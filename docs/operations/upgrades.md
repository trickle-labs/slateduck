# Upgrades

## Binary Upgrade

```bash
systemctl stop slateduck
cp slateduck-new /usr/local/bin/slateduck
systemctl start slateduck
```

Newer binaries always read older catalog formats. No manual migration needed.

## Catalog Format Version

Stored at `0xFF | catalog-format-version`. Newer binaries operate in compatibility mode on older formats and write new format for new keys.

## Rolling Upgrades (Kubernetes)

Update image tag. Kubernetes `Recreate` strategy stops old pod, starts new. Upgrade window: 5-15 seconds.
