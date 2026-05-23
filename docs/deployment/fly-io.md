# Fly.io Deployment

## Deploy

```toml
# fly.toml
app = "my-slateduck"
primary_region = "iad"

[build]
  dockerfile = "Dockerfile"

[env]
  AWS_REGION = "us-east-1"

[[services]]
  internal_port = 5432
  protocol = "tcp"
  [[services.ports]]
    port = 5432
```

```bash
fly secrets set AWS_ACCESS_KEY_ID=your-key AWS_SECRET_ACCESS_KEY=your-secret
fly deploy
```
