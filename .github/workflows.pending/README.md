# Workflows waiting to be activated

`ci.yml` and `release.yml` are complete, but they sit here instead of in
`.github/workflows/` because the GitHub connection that pushed this branch is
not allowed to create or change workflow files (it lacks the `workflow`
permission). Nothing in this folder runs.

To activate them, from a clone where you can push workflows:

```bash
git rm .github/workflows.pending/README.md
```

```bash
git mv .github/workflows.pending .github/workflows
```

```bash
git commit -m "Activate CI and release workflows"
```

```bash
git push
```

`ci.yml` then runs on every push and pull request; `release.yml` runs when a
`v*` tag is pushed. Neither needs any secret.
