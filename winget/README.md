# winget manifests

This folder holds the [winget](https://learn.microsoft.com/windows/package-manager/)
manifest sets for InSearch, one per release, under
`manifests/s/StruisICT/InSearch/<version>/` (package id **`StruisICT.InSearch`**,
schema 1.12.0). The installer is the released **MSI**; `InstallerSha256` and
`ProductCode` are read from the published asset.

## How they're generated

`scripts/Update-WingetManifest.ps1 -Version <x.y.z>` downloads the released MSI,
computes its SHA256, reads its ProductCode, and writes the three manifest files.
The **winget manifest** workflow runs this automatically after each release and
opens an in-repo PR with the new folder.

Validate locally with:

```powershell
winget validate --manifest winget/manifests/s/StruisICT/InSearch/<version>
```

## Submitting to winget (deliberate, manual)

These files are **not** submitted to `microsoft/winget-pkgs` automatically. When
you want a version published to the community repo:

1. Fork <https://github.com/microsoft/winget-pkgs>.
2. Copy `winget/manifests/s/StruisICT/InSearch/<version>/` into the fork under
   `manifests/s/StruisICT/InSearch/<version>/`.
3. Open a PR to `microsoft/winget-pkgs`. Their bot validates and (once a
   moderator approves) merges it.

After it merges, `winget install StruisICT.InSearch` works for everyone.
