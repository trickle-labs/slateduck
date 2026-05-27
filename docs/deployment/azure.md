# Deploying on Azure Blob Storage

Azure Blob Storage is an excellent backend for Rocklake deployments on Microsoft Azure. It provides sixteen-nines durability (with RA-GRS), native integration with Azure Active Directory, Managed Identity for zero-credential deployments on Azure compute, and competitive pricing for both storage and requests. This guide walks through setting up a Storage Account, configuring Azure RBAC permissions, authenticating Rocklake using Managed Identity or service principal credentials, and deploying on Azure Kubernetes Service (AKS), Azure Container Instances (ACI), and Azure Container Apps.

## Prerequisites

- An Azure subscription
- Azure CLI installed and authenticated (`az login`)
- Rocklake binary or Docker image
- Permissions to create Storage Accounts, resource groups, and Managed Identities (typically `Owner` or `Contributor` plus `User Access Administrator` on the subscription or resource group)

## Creating the Storage Account

Azure Blob Storage organizes storage into **Storage Accounts** and within them **containers** (equivalent to S3 buckets). Rocklake uses a container within a storage account as its catalog prefix.

```bash
# Create a resource group
az group create \
  --name rocklake-rg \
  --location eastus

# Create a storage account
# LRS = Locally Redundant Storage (3 copies within one datacenter) — cheapest
# ZRS = Zone-Redundant Storage (3 copies across 3 AZs) — recommended
# GRS = Geo-Redundant Storage (6 copies across 2 regions) — highest durability
az storage account create \
  --name mylakelhouse \
  --resource-group rocklake-rg \
  --location eastus \
  --sku Standard_ZRS \
  --kind StorageV2 \
  --min-tls-version TLS1_2 \
  --allow-blob-public-access false
```

For production, `Standard_ZRS` (Zone-Redundant Storage) provides a good balance of durability (twelve nines), performance, and cost. `Standard_GRS` adds cross-region replication for the highest durability requirements.

Create a container for the catalog:

```bash
az storage container create \
  --name catalog \
  --account-name mylakehouse \
  --auth-mode login
```

Create a separate container for data if you want bucket-level separation:

```bash
az storage container create \
  --name data \
  --account-name mylakehouse \
  --auth-mode login
```

## RBAC Configuration

Azure uses Role-Based Access Control (RBAC) for storage authorization. The relevant built-in roles for Blob Storage are:

| Role | Permissions | Use case |
|------|------------|---------|
| `Storage Blob Data Owner` | Full CRUD + ACLs | Not recommended — too broad |
| `Storage Blob Data Contributor` | Read, write, delete blobs | Rocklake catalog access |
| `Storage Blob Data Reader` | Read blobs only | DuckDB reader access |

### Managed Identity for Rocklake

Create a User-Assigned Managed Identity for Rocklake:

```bash
# Create the managed identity
az identity create \
  --name rocklake-identity \
  --resource-group rocklake-rg

# Get the principal ID for role assignment
PRINCIPAL_ID=$(az identity show \
  --name rocklake-identity \
  --resource-group rocklake-rg \
  --query principalId --output tsv)

# Get the storage account resource ID
STORAGE_ID=$(az storage account show \
  --name mylakehouse \
  --resource-group rocklake-rg \
  --query id --output tsv)

# Assign Storage Blob Data Contributor to the catalog container
# Using container-scoped assignment for least privilege
CONTAINER_SCOPE="${STORAGE_ID}/blobServices/default/containers/catalog"

az role assignment create \
  --assignee-object-id $PRINCIPAL_ID \
  --assignee-principal-type ServicePrincipal \
  --role "Storage Blob Data Contributor" \
  --scope $CONTAINER_SCOPE
```

### Service Principal for Non-Azure Environments

For deployments outside Azure (on-premises, other clouds), create a service principal:

```bash
# Create service principal with no initial role
SP_OUTPUT=$(az ad sp create-for-rbac \
  --name rocklake-sp \
  --skip-assignment \
  --output json)

APP_ID=$(echo $SP_OUTPUT | jq -r .appId)
APP_SECRET=$(echo $SP_OUTPUT | jq -r .password)
TENANT_ID=$(az account show --query tenantId --output tsv)

# Assign role to the catalog container
CONTAINER_SCOPE="${STORAGE_ID}/blobServices/default/containers/catalog"

az role assignment create \
  --assignee $APP_ID \
  --role "Storage Blob Data Contributor" \
  --scope $CONTAINER_SCOPE

echo "App ID: $APP_ID"
echo "Secret: $APP_SECRET (save this — it cannot be retrieved later)"
echo "Tenant ID: $TENANT_ID"
```

!!! warning "Secret management"
    Service principal secrets are long-lived credentials. Store them in Azure Key Vault and inject them into your runtime via Key Vault references or the Secrets Store CSI driver — do not hardcode them in configuration files or environment variables.

## Authentication Methods

Azure supports multiple authentication mechanisms, and the right choice depends on where Rocklake runs.

### Managed Identity (Recommended for Azure Compute)

Managed Identity is the zero-credential approach for workloads running on Azure Virtual Machines, AKS, ACI, or Container Apps. No secrets to rotate, no credentials to leak.

Set the environment variable to tell Rocklake to use Managed Identity:

```bash
# For System-Assigned Managed Identity:
export AZURE_USE_MANAGED_IDENTITY=true

# For User-Assigned Managed Identity (specify the client ID):
export AZURE_CLIENT_ID=your-managed-identity-client-id
```

### Service Principal with Client Secret

For environments outside Azure:

```bash
export AZURE_STORAGE_ACCOUNT_NAME=mylakehouse
export AZURE_CLIENT_ID=your-app-id
export AZURE_CLIENT_SECRET=your-app-secret
export AZURE_TENANT_ID=your-tenant-id
```

### Connection String (Development Only)

For local development and testing, you can use a storage account connection string. **Never use this in production** — connection strings grant full access to all containers in the storage account:

```bash
export AZURE_STORAGE_CONNECTION_STRING="DefaultEndpointsProtocol=https;AccountName=mylakehouse;AccountKey=...;EndpointSuffix=core.windows.net"
```

### Account Key (Development Only)

Similar caveats as connection strings:

```bash
export AZURE_STORAGE_ACCOUNT_NAME=mylakehouse
export AZURE_STORAGE_ACCOUNT_KEY="your-64-byte-base64-encoded-key=="
```

Retrieve the key with:

```bash
az storage account keys list \
  --account-name mylakehouse \
  --resource-group rocklake-rg \
  --query "[0].value" --output tsv
```

## Starting Rocklake

With credentials configured, start Rocklake pointing at your Azure Blob Storage container:

```bash
rocklake serve \
  --catalog az://mylakehouse/catalog/ \
  --bind 0.0.0.0:5432
```

Expected startup output:

```
INFO  Rocklake v0.8.0 starting
INFO  Storage backend: azure-blob
INFO  Catalog path: az://mylakehouse/catalog/
INFO  Opening SlateDB...
INFO  Using Managed Identity authentication
INFO  Catalog format version: v1
INFO  Next snapshot ID: 7
INFO  Writer epoch: 2
INFO  Listening on 0.0.0.0:5432
INFO  Ready to accept connections
```

## Deployment on Azure Kubernetes Service

Create a pod identity for AKS using Azure Workload Identity (the modern replacement for Pod Identity v1):

```bash
# Enable Workload Identity on the cluster
az aks update \
  --name my-aks-cluster \
  --resource-group rocklake-rg \
  --enable-oidc-issuer \
  --enable-workload-identity

# Get the OIDC issuer URL
OIDC_ISSUER=$(az aks show \
  --name my-aks-cluster \
  --resource-group rocklake-rg \
  --query "oidcIssuerProfile.issuerUrl" --output tsv)

# Get the managed identity client ID
CLIENT_ID=$(az identity show \
  --name rocklake-identity \
  --resource-group rocklake-rg \
  --query clientId --output tsv)

# Create the federated identity credential linking the Kubernetes service account
az identity federated-credential create \
  --name rocklake-federated-credential \
  --identity-name rocklake-identity \
  --resource-group rocklake-rg \
  --issuer $OIDC_ISSUER \
  --subject system:serviceaccount:rocklake-ns:rocklake-sa \
  --audience api://AzureADTokenExchange
```

Kubernetes deployment manifest:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: rocklake-sa
  namespace: rocklake-ns
  annotations:
    azure.workload.identity/client-id: "your-managed-identity-client-id"
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rocklake
  namespace: rocklake-ns
spec:
  replicas: 1
  selector:
    matchLabels:
      app: rocklake
  template:
    metadata:
      labels:
        app: rocklake
        azure.workload.identity/use: "true"  # Enable workload identity
    spec:
      serviceAccountName: rocklake-sa
      containers:
        - name: rocklake
          image: ghcr.io/rocklake/rocklake:latest
          args:
            - serve
            - --catalog
            - az://mylakehouse/catalog/
            - --bind
            - 0.0.0.0:5432
          env:
            - name: AZURE_CLIENT_ID
              value: "your-managed-identity-client-id"
          ports:
            - containerPort: 5432
```

## Deployment on Azure Container Apps

Container Apps supports Managed Identity natively:

```bash
# Create the Container App environment
az containerapp env create \
  --name rocklake-env \
  --resource-group rocklake-rg \
  --location eastus

# Get the managed identity resource ID
IDENTITY_ID=$(az identity show \
  --name rocklake-identity \
  --resource-group rocklake-rg \
  --query id --output tsv)

# Create the Container App with the managed identity
az containerapp create \
  --name rocklake \
  --resource-group rocklake-rg \
  --environment rocklake-env \
  --image ghcr.io/rocklake/rocklake:latest \
  --target-port 5432 \
  --ingress external \
  --user-assigned $IDENTITY_ID \
  --command '["rocklake", "serve", "--catalog", "az://mylakehouse/catalog/", "--bind", "0.0.0.0:5432"]' \
  --env-vars AZURE_CLIENT_ID=your-managed-identity-client-id \
  --min-replicas 1 \
  --max-replicas 1
```

The `--min-replicas 1` setting is important for the same reason as Cloud Run's minimum instances: Rocklake holds a writer lock and must not scale to zero.

## Connecting DuckDB

```sql
INSTALL ducklake;
LOAD ducklake;

ATTACH 'ducklake:host=rocklake.internal;port=5432' AS lake;

-- Verify access
SHOW ALL TABLES FROM lake;
```

DuckDB's own Azure credentials for direct Parquet file access are configured separately using DuckDB's Azure extension (`INSTALL azure; LOAD azure;`) with the appropriate connection string or Managed Identity configuration.

## Performance Characteristics

Azure Blob Storage offers performance comparable to AWS S3 Standard:

| Operation | Azure Blob (Hot) | Azure Blob (Cool) | Notes |
|-----------|-----------------|-------------------|-------|
| PUT (WAL segment) | 20–70 ms | Higher retrieval cost | Hot tier preferred |
| GET (SST block) | 10–50 ms | 50–120 ms | Hot tier for active catalogs |
| LIST | 30–80 ms | 30–80 ms | Same across tiers |

Use the **Hot access tier** for Rocklake catalogs. The Cool and Archive tiers impose retrieval latency and minimum storage duration charges that make them unsuitable for active catalog storage.

Enable the **ZRS redundancy tier** (`Standard_ZRS`) for zone-fault tolerance with catalog data. This provides twelve nines durability (99.9999999999%) — adequate for nearly all production requirements.

## Monitoring and Troubleshooting

**Check bucket connectivity:**

```bash
az storage blob list \
  --account-name mylakehouse \
  --container-name catalog \
  --auth-mode login
```

**Enable diagnostic logging:**

```bash
az monitor diagnostic-settings create \
  --name rocklake-catalog-diag \
  --resource $STORAGE_ID \
  --storage-account $STORAGE_ID \
  --logs '[{"category": "StorageRead", "enabled": true}, {"category": "StorageWrite", "enabled": true}]' \
  --metrics '[{"category": "Transaction", "enabled": true}]'
```

**Common errors:**

| Error | Cause | Fix |
|-------|-------|-----|
| `AuthorizationPermissionMismatch` | IAM role not assigned | Check role assignment scope; ensure container scope, not resource group |
| `The specified resource does not exist` | Container not created | Run `az storage container create` |
| `This request is not authorized to perform this operation` | Account key access with Managed Identity disabled | Enable Managed Identity or check key authentication settings |
| `NetworkAccessDenied` | Firewall rule blocking access | Add an exception for your VM/container's IP or VNet |

## Further Reading

- **[Credential Isolation](credential-isolation.md)** — Separate Managed Identities for catalog and data plane access
- **[Kubernetes Deployment](kubernetes.md)** — Full AKS deployment guide with Azure Workload Identity
- **[Object Store Durability](../concepts/object-store-durability.md)** — Durability model of object storage backends
