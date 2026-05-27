# TLS Configuration

Rocklake supports TLS encryption for all client connections, protecting catalog metadata in transit between DuckDB and the Rocklake server. TLS is essential for any deployment where the network between client and server is not fully trusted — which in practice means every deployment except localhost development. Even within a VPC, defense in depth argues for encrypting the wire.

This page covers enabling TLS, certificate management strategies, mutual TLS for zero-trust environments, TLS termination at load balancers, certificate rotation, and integration with DuckDB clients.

## Why TLS Matters for Rocklake

The PostgreSQL wire protocol that Rocklake implements transmits data in plaintext by default. Without TLS, an attacker with network access can:

- Read catalog metadata (table names, schemas, column types) — information leakage
- Capture authentication credentials (username/password) — credential theft
- Modify queries in transit (man-in-the-middle) — data integrity risk
- Replay captured sessions — unauthorized access

TLS eliminates all of these attack vectors with standard, well-understood cryptography.

## Enabling TLS

### Basic Configuration

Provide a certificate and private key:

```bash
rocklake \
    --catalog s3://bucket/catalog/ \
    --bind 0.0.0.0:5432 \
    --tls-cert /etc/rocklake/tls/server.crt \
    --tls-key /etc/rocklake/tls/server.key
```

Or via environment variables:

```bash
export ROCKLAKE_TLS_CERT=/etc/rocklake/tls/server.crt
export ROCKLAKE_TLS_KEY=/etc/rocklake/tls/server.key
rocklake serve --catalog s3://bucket/catalog/ --bind 0.0.0.0:5432
```

### TLS Behavior

When TLS is configured, Rocklake implements the PostgreSQL SSL negotiation:

1. Client sends `SSLRequest` message
2. Rocklake responds with `S` (SSL supported)
3. TLS handshake occurs
4. All subsequent protocol messages are encrypted

Clients that do not request SSL receive `S` anyway — the server always advertises TLS availability. If you want to **require** TLS (reject plaintext connections), add:

```bash
rocklake --tls-cert ... --tls-key ... --tls-required
```

With `--tls-required`, clients that attempt plaintext connections receive an error and are disconnected.

### Supported TLS Versions

Rocklake supports TLS 1.2 and TLS 1.3. TLS 1.0 and 1.1 are disabled (they have known vulnerabilities). The server uses `rustls` (a modern, memory-safe TLS library) rather than OpenSSL.

### Cipher Suites

Rocklake uses rustls defaults, which prioritize:

- TLS 1.3: `TLS_AES_256_GCM_SHA384`, `TLS_AES_128_GCM_SHA256`, `TLS_CHACHA20_POLY1305_SHA256`
- TLS 1.2: `TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384`, `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256`

These are considered secure as of 2024. Weak ciphers (RC4, DES, export-grade) are not available.

## Certificate Sources

### Self-Signed Certificates (Development Only)

Generate a self-signed certificate for local development and testing:

```bash
# Generate private key and self-signed certificate
openssl req -x509 -newkey rsa:4096 \
    -keyout server.key \
    -out server.crt \
    -days 365 \
    -nodes \
    -subj '/CN=localhost' \
    -addext 'subjectAltName=DNS:localhost,IP:127.0.0.1'
```

For development with Docker Compose (SAN includes service name):

```bash
openssl req -x509 -newkey rsa:4096 \
    -keyout server.key \
    -out server.crt \
    -days 365 \
    -nodes \
    -subj '/CN=rocklake' \
    -addext 'subjectAltName=DNS:rocklake,DNS:localhost,IP:127.0.0.1'
```

DuckDB must trust this certificate. For development, you can set `sslmode=require` (which skips CA verification) rather than `sslmode=verify-full`.

### Let's Encrypt (Production — Public Endpoints)

For internet-facing Rocklake instances with a public DNS name:

```bash
# Using certbot standalone mode
certbot certonly --standalone \
    -d catalog.example.com \
    --cert-path /etc/rocklake/tls/server.crt \
    --key-path /etc/rocklake/tls/server.key
```

Configure automatic renewal:

```bash
# Crontab entry
0 3 * * * certbot renew --quiet --deploy-hook "systemctl reload rocklake"
```

Rocklake watches the certificate file for changes and reloads automatically without restarting (zero-downtime renewal).

### Private CA (Enterprise / Internal)

For organizations with an internal PKI:

```bash
# Generate server key
openssl genrsa -out server.key 4096

# Generate CSR
openssl req -new -key server.key -out server.csr \
    -subj '/CN=rocklake.internal.example.com/O=MyCompany'

# Sign with internal CA
openssl x509 -req -in server.csr \
    -CA /etc/pki/ca.crt \
    -CAkey /etc/pki/ca.key \
    -CAcreateserial \
    -out server.crt \
    -days 365 \
    -extfile <(printf "subjectAltName=DNS:rocklake.internal.example.com")
```

### Cloud Certificate Managers

For managed certificate lifecycle:

**AWS Certificate Manager (ACM):**
ACM certificates cannot be exported for direct use. Instead, terminate TLS at an NLB/ALB with the ACM certificate, and run Rocklake in plaintext behind it. See [TLS Termination at Load Balancer](#tls-termination-at-load-balancer).

**HashiCorp Vault PKI:**
```bash
# Issue a certificate from Vault
vault write pki/issue/rocklake-role \
    common_name="rocklake.internal" \
    ttl="720h" \
    -format=json | jq -r '.data.certificate' > server.crt
```

**cert-manager (Kubernetes):**
```yaml
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: rocklake-tls
  namespace: rocklake
spec:
  secretName: rocklake-tls
  issuerRef:
    name: letsencrypt-prod
    kind: ClusterIssuer
  dnsNames:
    - rocklake.example.com
    - rocklake.internal.svc.cluster.local
```

Mount the secret into the pod:

```yaml
volumes:
  - name: tls
    secret:
      secretName: rocklake-tls
containers:
  - name: rocklake
    volumeMounts:
      - name: tls
        mountPath: /etc/rocklake/tls
        readOnly: true
```

## Mutual TLS (mTLS)

Mutual TLS requires clients to present a certificate that the server verifies. This provides strong client authentication without passwords — the client proves its identity cryptographically.

### Enabling mTLS

```bash
rocklake \
    --catalog s3://bucket/catalog/ \
    --bind 0.0.0.0:5432 \
    --tls-cert /etc/rocklake/tls/server.crt \
    --tls-key /etc/rocklake/tls/server.key \
    --tls-ca /etc/rocklake/tls/client-ca.crt
```

The `--tls-ca` flag specifies the Certificate Authority that signed the client certificates. Only clients presenting a certificate signed by this CA will be allowed to connect.

### Generating Client Certificates

```bash
# Generate client key
openssl genrsa -out client.key 4096

# Generate client CSR
openssl req -new -key client.key -out client.csr \
    -subj '/CN=analytics-team/O=MyCompany'

# Sign with client CA
openssl x509 -req -in client.csr \
    -CA client-ca.crt \
    -CAkey client-ca.key \
    -CAcreateserial \
    -out client.crt \
    -days 365
```

### Connecting with Client Certificate (DuckDB)

```sql
ATTACH 'ducklake:host=rocklake.example.com;port=5432;sslmode=verify-full;sslcert=/path/to/client.crt;sslkey=/path/to/client.key;sslrootcert=/path/to/server-ca.crt' AS lake;
```

### Use Cases for mTLS

- **Zero-trust networks** — where password-based auth is insufficient
- **Service-to-service** — automated systems (ETL pipelines, CI/CD) with rotatable certificates
- **Multi-tenant isolation** — different teams get different client certificates, enabling audit trails
- **Compliance** — regulations requiring strong authentication (PCI-DSS, SOC2)

## TLS Termination at Load Balancer

For deployments behind a load balancer or reverse proxy, it is common to terminate TLS at the load balancer and run plaintext between the LB and Rocklake:

```
DuckDB ──(TLS)──→ Load Balancer ──(plaintext)──→ Rocklake
```

This is acceptable when:

- The LB-to-Rocklake network is trusted (same host, same pod, service mesh)
- The load balancer handles certificate management and renewal
- You want to offload cryptographic work from Rocklake

### AWS NLB with TLS Termination

```yaml
apiVersion: v1
kind: Service
metadata:
  name: rocklake
  annotations:
    service.beta.kubernetes.io/aws-load-balancer-type: nlb
    service.beta.kubernetes.io/aws-load-balancer-ssl-cert: arn:aws:acm:us-east-1:123456:certificate/abc-123
    service.beta.kubernetes.io/aws-load-balancer-ssl-ports: "5432"
    service.beta.kubernetes.io/aws-load-balancer-backend-protocol: tcp
spec:
  type: LoadBalancer
  ports:
    - port: 5432
      targetPort: 5432
```

### End-to-End TLS (Preferred for Compliance)

For environments requiring end-to-end encryption, run TLS both at the LB and within Rocklake:

```
DuckDB ──(TLS)──→ Load Balancer ──(TLS)──→ Rocklake
```

The LB performs TLS pass-through (Layer 4 forwarding) and Rocklake handles the TLS handshake directly.

## Certificate Rotation

### Zero-Downtime Rotation

Rocklake monitors certificate files for changes. When the certificate or key file is modified:

1. Rocklake detects the file change (via filesystem notification)
2. New connections use the new certificate
3. Existing connections continue with the old certificate until they disconnect
4. No restart required

### Rotation Procedure

```bash
# 1. Obtain new certificate
certbot renew

# 2. Copy to Rocklake's certificate directory
cp /etc/letsencrypt/live/catalog.example.com/fullchain.pem /etc/rocklake/tls/server.crt
cp /etc/letsencrypt/live/catalog.example.com/privkey.pem /etc/rocklake/tls/server.key

# 3. Rocklake picks up the change automatically (no restart needed)
```

### Kubernetes Secret Rotation

With cert-manager, certificate rotation is fully automatic:

1. cert-manager renews the certificate before expiry
2. Kubernetes updates the Secret
3. The kubelet updates the mounted file in the pod
4. Rocklake detects the file change and loads the new certificate

## DuckDB Client Configuration

### SSL Modes

| Mode | Behavior | Use Case |
|------|----------|----------|
| `disable` | No TLS | Local development only |
| `allow` | Try TLS, fall back to plaintext | Not recommended |
| `prefer` | Try TLS, fall back to plaintext | Default in many clients |
| `require` | TLS required, no CA verification | Development with self-signed certs |
| `verify-ca` | TLS + verify server certificate CA | Internal PKI |
| `verify-full` | TLS + verify CA + hostname match | Production (recommended) |

### Connection Strings

```sql
-- Require TLS (no certificate verification)
ATTACH 'ducklake:host=rocklake.example.com;port=5432;sslmode=require' AS lake;

-- Full verification (production recommended)
ATTACH 'ducklake:host=rocklake.example.com;port=5432;sslmode=verify-full;sslrootcert=/path/to/ca.crt' AS lake;

-- Mutual TLS
ATTACH 'ducklake:host=rocklake.example.com;port=5432;sslmode=verify-full;sslrootcert=/path/to/ca.crt;sslcert=/path/to/client.crt;sslkey=/path/to/client.key' AS lake;
```

## Troubleshooting TLS

### "SSL connection has been closed unexpectedly"

- Server certificate expired — check with `openssl x509 -in cert.pem -noout -dates`
- Key does not match certificate — verify with `openssl x509 -in cert.pem -modulus -noout | md5` vs `openssl rsa -in key.pem -modulus -noout | md5`

### "certificate verify failed"

- Client does not trust the server's CA — add the CA to the client's trust store or use `sslrootcert`
- Hostname mismatch — ensure the certificate's SAN includes the hostname used in the connection string

### "no suitable TLS certificate found for client"

- mTLS is configured but client did not provide a certificate
- Client certificate not signed by the expected CA

### Checking TLS Status

```bash
# Verify TLS is working
openssl s_client -connect rocklake.example.com:5432 -starttls postgres

# Check certificate details
echo | openssl s_client -connect rocklake.example.com:5432 -starttls postgres 2>/dev/null | openssl x509 -noout -text
```

## Authentication Strategies

TLS encrypts the channel; authentication determines who can use it. Rocklake supports three authentication models, which can be combined.

### Password Authentication

The simplest model: DuckDB provides a password when connecting, and Rocklake verifies it. Enable by setting `--auth-user` and `--auth-password` (or `ROCKLAKE_AUTH_PASSWORD`):

```bash
rocklake serve \
  --catalog s3://my-bucket/catalog/ \
  --bind 0.0.0.0:5432 \
  --auth-user ducklake \
  --tls-cert /etc/rocklake/tls/server.crt \
  --tls-key /etc/rocklake/tls/server.key
```

DuckDB connects with:

```sql
ATTACH 'ducklake:host=rocklake.example.com;port=5432;user=ducklake;password=my-token;sslmode=require' AS lake;
```

**Important:** Password authentication provides no real security without TLS. The password would be transmitted in plaintext and trivially intercepted. Always combine password authentication with at least `sslmode=require`.

### Certificate-Only Authentication (mTLS)

With mutual TLS configured, the client certificate itself proves identity — no password is needed. This is the preferred model for service-to-service connections (ETL pipelines, automated jobs) because:

- Certificates can be rotated without code changes
- No shared secret to accidentally expose in logs or config files
- Certificate identity is auditable (the CN/SAN identifies the connecting service)

To use certificate-only auth, configure mTLS and do not set `--auth-password`:

```bash
rocklake serve \
  --catalog s3://my-bucket/catalog/ \
  --bind 0.0.0.0:5432 \
  --tls-cert server.crt \
  --tls-key server.key \
  --tls-ca client-ca.crt
  # No --auth-password: certificate IS the authentication
```

### Combined: Certificate + Password

For environments requiring two factors, configure both mTLS and password authentication. A connecting client must present a valid certificate AND the correct password:

```bash
rocklake serve \
  --catalog s3://my-bucket/catalog/ \
  --bind 0.0.0.0:5432 \
  --tls-cert server.crt \
  --tls-key server.key \
  --tls-ca client-ca.crt \
  --auth-user ducklake
  # ROCKLAKE_AUTH_PASSWORD set in environment
```

This satisfies strict compliance requirements (PCI-DSS, HIPAA) that mandate multi-factor authentication for privileged system access.

### No Authentication (Development Only)

In local development, running Rocklake without any authentication is fine:

```bash
# Local development: no TLS, no auth
rocklake serve --catalog ./dev-catalog --bind 127.0.0.1:5432
```

Never expose an unauthenticated Rocklake instance on a non-loopback address. Even within a private network, the absence of authentication means any host on that network can modify your catalog.

## TLS in Containerized Environments

### Docker Compose

Mount certificates as a read-only volume:

```yaml
services:
  rocklake:
    image: ghcr.io/rocklake/rocklake:latest
    command: >
      serve
      --catalog s3://my-bucket/catalog/
      --bind 0.0.0.0:5432
      --tls-cert /etc/rocklake/tls/server.crt
      --tls-key /etc/rocklake/tls/server.key
    volumes:
      - ./certs:/etc/rocklake/tls:ro
    environment:
      ROCKLAKE_AUTH_PASSWORD: "${AUTH_PASSWORD}"
      AWS_REGION: us-east-1
```

Generate the certificates once during local setup:

```bash
mkdir -p ./certs
openssl req -x509 -newkey rsa:4096 -keyout ./certs/server.key \
  -out ./certs/server.crt -days 365 -nodes \
  -subj '/CN=rocklake' \
  -addext 'subjectAltName=DNS:rocklake,DNS:localhost,IP:127.0.0.1'
```

### Reading Cert Paths from Secrets

For production Docker deployments, use Docker Secrets or environment variables pointing at mounted secret files — never bake certificate paths into the image:

```yaml
services:
  rocklake:
    environment:
      ROCKLAKE_TLS_CERT: /run/secrets/server_crt
      ROCKLAKE_TLS_KEY: /run/secrets/server_key
    secrets:
      - server_crt
      - server_key

secrets:
  server_crt:
    file: ./certs/server.crt
  server_key:
    file: ./certs/server.key
```

## Further Reading

- **[Networking](networking.md)** — Network topology and firewall configuration
- **[Configuration](configuration.md)** — TLS-related configuration options
- **[Kubernetes](kubernetes.md)** — cert-manager integration
- **[High Availability](high-availability.md)** — TLS with load balancers and failover
- **[Credential Isolation](credential-isolation.md)** — IAM/RBAC separation for catalog and data planes

## TLS Quick-Reference Checklist

Use this checklist before going to production:

- [ ] Server certificate has the correct hostname in its Subject Alternative Name (SAN)
- [ ] Certificate is signed by a CA that your DuckDB clients trust
- [ ] Private key file has restricted permissions (`chmod 600 server.key`)
- [ ] `sslmode=require` or stronger configured on all DuckDB connection strings
- [ ] `--tls-required` enabled if you want to reject plaintext connections entirely
- [ ] Certificate expiry monitoring in place (alert at 30 days, critical at 7 days)
- [ ] Certificate rotation tested in staging before production rollout
- [ ] For mTLS: client CA certificate distributed to all Rocklake instances
- [ ] For mTLS: each automated client has a unique CN/SAN for audit purposes
- [ ] Authentication password set via environment variable, not command-line flag
