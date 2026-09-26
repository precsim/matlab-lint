# Releasing mstyle

Binary releases are created from version tags matching `v*`.

Example:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The release workflow builds native release binaries on Linux, Windows, and macOS. Each binary is staged with the runner OS and architecture in its filename and uploaded to the corresponding GitHub Release.

The normal CI workflow separately verifies release builds on Windows and macOS for every push and pull request.
