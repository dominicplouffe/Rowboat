# A workflow change waiting to be applied

`ci.yml` here is the next version of `.github/workflows/ci.yml`. It sits in this
folder because the GitHub connection that pushed it may not change workflow
files. Nothing in this folder runs.

What it changes:

- Adds a `publish` job: every green push to `main` replaces a rolling
  pre-release called `latest-build` with the installers CI just built.
- Raises the macOS cold-start limit for hosted runners from 2.5 s to 5 s. The
  same build measured between 1.5 s and 4.8 s on them.

To apply it, from a clone where you can push workflows:

```bash
git mv -f .github/workflows.pending/ci.yml .github/workflows/ci.yml
```

```bash
git rm .github/workflows.pending/README.md
```

```bash
git commit -m "CI: publish a rolling latest-build pre-release"
```

```bash
git push
```
