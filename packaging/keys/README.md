# Skemer release signing key

Public material for the OpenPGP key that signs Skemer release artifacts. This
key is separate from the muis and Veldmuis release keys.

The private key never lives in this repository. The primary key is held offline;
release automation receives a passphrase-protected signing key through the
protected `release` GitHub Environment.

## Files

The release workflow expects these files before it will publish:

- `skemer-release.gpg` — binary keyring, used with `gpgv --keyring`.
- `skemer-release.asc` — armored copy for humans and release attachments.
- `skemer-trusted` — the primary fingerprint to pin against.

They are not generated automatically. Create them once, verify them, and commit
only the public material.

## Create the key

Run this on a trusted machine, not in CI:

```sh
gpg --quick-generate-key \
  "Skemer release signing <skemer@veldmuislinux.org>" ed25519 cert 5y

fpr="$(gpg --list-keys --with-colons skemer@veldmuislinux.org \
  | awk -F: '$1 == "fpr" { print $10; exit }')"

gpg --quick-add-key "${fpr}" ed25519 sign 5y

gpg --export "${fpr}" > packaging/keys/skemer-release.gpg
gpg --armor --export "${fpr}" > packaging/keys/skemer-release.asc
printf '%s\n' "${fpr}" > packaging/keys/skemer-trusted
```

Export the protected signing subkey for the GitHub Environment secret:

```sh
gpg --armor --export-secret-subkeys "${fpr}" > /secure/skemer-release-key.asc
```

Store the passphrase separately. Add three secrets to the `release` environment:

```text
SKEMER_GPG_PRIVATE_KEY   armored secret key
SKEMER_GPG_PASSPHRASE    key passphrase
SKEMER_GPG_FPR           primary fingerprint from skemer-trusted
```

## Verify a release artifact

Every artifact is published with a detached armored signature, and the aggregate
`SHA256SUMS` is signed too.

```sh
gpgv --keyring packaging/keys/skemer-release.gpg \
  skemer-<version>-x86_64-linux.tar.gz.asc \
  skemer-<version>-x86_64-linux.tar.gz

gpgv --keyring packaging/keys/skemer-release.gpg SHA256SUMS.asc SHA256SUMS
sha256sum -c SHA256SUMS
```

## Rotation

Routine change is a new signing subkey under the same primary fingerprint.
Replacing the primary fingerprint is a staged, reviewed transition. A suspected
compromise follows the same emergency response as any signing key: stop signing,
preserve evidence, revoke from the offline primary, and publish an authenticated
notice before resuming.

The Veldmuis package vendor must update `development/skemer-keyring/` and the
package pin after a primary-key rotation.
