**Profile: infrastructure / DevOps**

- **Never run `terraform apply`, `kubectl delete`, or equivalent destructive commands** without showing the plan/diff and receiving explicit confirmation.
- **Plan before apply**: always run `terraform plan` or `kubectl diff` first and surface the output.
- **Prefer declarative config** — Terraform HCL, Kubernetes manifests, Helm values — over imperative CLI sequences.
- **Secrets**: reference vault paths or secret manager keys; never hardcode or echo credentials.
- **Every change must include a rollback strategy** — previous state, revert command, or rollback manifest.
