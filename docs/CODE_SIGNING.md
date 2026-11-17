# Code signing

## Status

The downloads are not signed yet. The release workflow is ready to sign the Windows programs
through [SignPath.io](https://about.signpath.io) once a certificate is in place; until then, check
a download with its SHA-256 sum and build attestation (below).

## Policy

- **What is signed.** Only what this repository's release workflow
  ([`.github/workflows/release.yml`](../.github/workflows/release.yml)) builds from the tagged
  source: `keyhold.exe`, `keyhold-launch.exe` and the installer `KeyholdSetup.exe`. Nothing built
  on a person's machine is ever signed.
- **Committers and reviewers:** [Daniel Barcelos](https://github.com/danielbbarcelos).
- **Approvers:** [Daniel Barcelos](https://github.com/danielbbarcelos). Every signing request for
  a release is approved by hand in SignPath.
- Accounts with access to the repository and to SignPath use multi-factor authentication.

## Privacy

This program will not transfer any information to other networked systems unless specifically
requested by the user or the person installing or operating it. Keyhold talks to the network only
when the person connects an AWS account to it, and then only to AWS. See
[What Keyhold promises](../README.md#what-keyhold-promises-and-what-it-doesnt).

## Checking a download

```powershell
Get-AuthenticodeSignature .\KeyholdSetup.exe   # once signing is in place: Status Valid
```

Every release also carries a SHA-256 list and a build attestation:

```bash
gh attestation verify KeyholdSetup.exe --repo danielbbarcelos/keyhold
```

## How the workflow signs

Once SignPath has accepted the project, the repository holds:

| Kind | Name | Value |
|---|---|---|
| variable | `SIGNPATH_ORGANIZATION_ID` | the organization id SignPath gives the project |
| secret | `SIGNPATH_API_TOKEN` | the token of the CI user SignPath sets up |

The project in SignPath is `keyhold`, the signing policy `release-signing`, and the artifact
configurations `programs` and `installer` are the files in
[`packaging/windows/signpath/`](../packaging/windows/signpath/). A release tag then signs the two
programs, builds the installer from them, signs the installer and checks all three signatures
before publishing. Without the variable, or on a manual run, the workflow builds unsigned.

The macOS app will be signed and notarized with a Developer ID; that is not set up yet.
