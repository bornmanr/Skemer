#!/usr/bin/env bash

# Guided setup for the Skemer release key.
#
# This script generates or reuses a separate Skemer release key, writes the
# public key files into the Skemer and Veldmuis checkouts, exports the secret
# key material outside both repositories, and optionally sets the GitHub
# release environment secrets.
#
# It never commits or pushes. Review the written files and run the printed git
# commands yourself.

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skemer_repo="${SKEMER_REPO:-$(cd "${script_dir}/.." && pwd)}"
veldmuis_repo="${VELDMUIS_REPO:-${HOME}/Projects/veldmuis}"
secrets_dir="${SKEMER_SECRETS_DIR:-${HOME}/skemer-release-secrets}"
uid="${SKEMER_KEY_UID:-Skemer release signing <skemer@veldmuislinux.org>}"
expire="${SKEMER_KEY_EXPIRE:-5y}"
repo_slug="ruannnebornman/Skemer"
test_file=""
test_sig=""

cleanup_test() {
  [[ -z "${test_file}" ]] || rm -f -- "${test_file}"
  [[ -z "${test_sig}" ]] || rm -f -- "${test_sig}"
}

trap cleanup_test EXIT

log() {
  printf '[skemer-key] %s\n' "$*"
}

die() {
  printf '[skemer-key] ERROR: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Missing required command: $1"
}

expand_home() {
  # shellcheck disable=SC2088  # the leading ~/ is a literal match
  case "$1" in
    "~/"*) printf '%s/%s\n' "${HOME}" "${1#\~/}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

ask_yes() {
  local prompt="$1" answer=""
  read -r -p "${prompt} [Y/n] " answer
  [[ -z "${answer}" || "${answer}" =~ ^[Yy]([Ee][Ss])?$ ]]
}

ask_no() {
  local prompt="$1" answer=""
  read -r -p "${prompt} [y/N] " answer
  [[ "${answer}" =~ ^[Yy]([Ee][Ss])?$ ]]
}

usage() {
  cat <<'EOF'
Usage:
  setup-release-key.sh
  setup-release-key.sh --help

Environment overrides:
  SKEMER_REPO         Default: the repository containing this script
  VELDMUIS_REPO       Default: ~/Projects/veldmuis
  SKEMER_SECRETS_DIR  Default: ~/skemer-release-secrets
  SKEMER_KEY_UID      Default: Skemer release signing <skemer@veldmuislinux.org>
  SKEMER_KEY_EXPIRE   Default: 5y

The script writes public key files into the Skemer and Veldmuis checkouts,
exports secret material outside both repositories, and optionally sets the
GitHub release environment secrets. It never commits or pushes.
EOF
}

main() {
  case "${1:-}" in
    -h | --help)
      usage
      exit 0
      ;;
    "")
      ;;
    *)
      die "Unknown argument: $1"
      ;;
  esac

  [[ -t 0 ]] || die "Run this in a terminal; it needs interactive passphrase prompts."

  require_cmd gpg
  require_cmd gh
  require_cmd git
  require_cmd awk
  require_cmd sed
  require_cmd mktemp
  require_cmd install

  gh auth status >/dev/null 2>&1 || die "Run gh auth login first."

  log "The Skemer release key is separate from the muis and Veldmuis keys."
  log "Do not reuse the muis key for Skemer releases."

  read -r -p "Veldmuis repository [${veldmuis_repo}]: " input
  [[ -z "${input}" ]] || veldmuis_repo="${input}"
  veldmuis_repo="$(expand_home "${veldmuis_repo}")"

  read -r -p "Secrets directory (outside both repositories) [${secrets_dir}]: " input
  [[ -z "${input}" ]] || secrets_dir="${input}"
  secrets_dir="$(expand_home "${secrets_dir}")"

  skemer_repo="$(expand_home "${skemer_repo}")"

  [[ -d "${skemer_repo}" ]] || die "Skemer repository not found: ${skemer_repo}"
  [[ -d "${veldmuis_repo}" ]] || die "Veldmuis repository not found: ${veldmuis_repo}"
  git -C "${skemer_repo}" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
    || die "Not a git checkout: ${skemer_repo}"
  git -C "${veldmuis_repo}" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
    || die "Not a git checkout: ${veldmuis_repo}"

  [[ -f "${skemer_repo}/.github/workflows/release.yml" ]] \
    || die "The release workflow is not in ${skemer_repo}; use the packaging PR branch or merge it first."
  [[ -f "${veldmuis_repo}/packages/veldmuis-skemer/PKGBUILD" ]] \
    || die "The veldmuis-skemer package is not in ${veldmuis_repo}; use the packaging PR branch or merge it first."

  case "${secrets_dir}" in
    "${skemer_repo}"/* | "${veldmuis_repo}"/*)
      die "Do not put the secrets directory inside either repository: ${secrets_dir}"
      ;;
  esac

  local skemer_branch veldmuis_branch
  skemer_branch="$(git -C "${skemer_repo}" branch --show-current)"
  veldmuis_branch="$(git -C "${veldmuis_repo}" branch --show-current)"

  log "Skemer checkout:  ${skemer_repo} (branch: ${skemer_branch:-detached})"
  log "Veldmuis checkout: ${veldmuis_repo} (branch: ${veldmuis_branch:-detached})"
  log "Secrets directory: ${secrets_dir}"
  ask_yes "Use these checkouts?" || die "Aborted."

  if [[ "${skemer_branch}" == "main" || "${veldmuis_branch}" == "main" ]]; then
    log "WARNING: one checkout is on main. Create a branch before committing the public key files."
  fi

  install -d -m 700 "${secrets_dir}"
  chmod 700 "${secrets_dir}"

  local existing_fpr fpr
  existing_fpr="$(gpg --list-secret-keys --with-colons "${uid}" 2>/dev/null \
    | awk -F: '$1 == "fpr" { print $10; exit }')"

  if [[ -n "${existing_fpr}" ]]; then
    log "Found an existing Skemer release key:"
    gpg --list-secret-keys --keyid-format=long "${existing_fpr}" || true
    ask_yes "Use this existing key?" || die "Aborted; move or revoke the old key before creating another one."
    fpr="${existing_fpr}"
  else
    ask_yes "Generate a new Skemer release key for '${uid}' (${expire} expiry)?" \
      || die "Aborted."
    log "GPG will prompt for the new key passphrase. Save it in your vault now."
    gpg --quick-generate-key "${uid}" ed25519 cert "${expire}"
    fpr="$(gpg --list-keys --with-colons "${uid}" \
      | awk -F: '$1 == "fpr" { print $10; exit }')"
    [[ -n "${fpr}" ]] || die "Could not read the new key fingerprint."
    log "Adding the sign-only subkey. GPG may prompt for the passphrase again."
    gpg --quick-add-key "${fpr}" ed25519 sign "${expire}"
  fi

  log "Skemer release key fingerprint: ${fpr}"

  local pass1 pass2
  read -r -s -p "Skemer release key passphrase (single line): " pass1
  echo
  read -r -s -p "Enter the passphrase again: " pass2
  echo
  [[ -n "${pass1}" ]] || die "The passphrase cannot be empty."
  [[ "${pass1}" == "${pass2}" ]] || die "The passphrases do not match."
  unset pass2

  test_file="$(mktemp)"
  test_sig="$(mktemp)"
  printf 'skemer release key test\n' > "${test_file}"
  if ! printf '%s' "${pass1}" \
    | gpg --batch --yes --pinentry-mode loopback --passphrase-fd 0 \
      --local-user "${fpr}" --detach-sign -o "${test_sig}" "${test_file}" 2>/dev/null
  then
    die "The passphrase does not unlock the Skemer signing key."
  fi
  gpg --batch --verify "${test_sig}" "${test_file}" >/dev/null 2>&1 \
    || die "The test signature did not verify."
  log "Passphrase verified against the signing key."

  local keys_dir="${skemer_repo}/packaging/keys"
  mkdir -p "${keys_dir}"

  if [[ -r "${keys_dir}/skemer-trusted" ]]; then
    local old_fpr
    old_fpr="$(tr -d '[:space:]' < "${keys_dir}/skemer-trusted" | tr '[:lower:]' '[:upper:]')"
    if [[ "${old_fpr}" != "${fpr}" ]]; then
      ask_yes "Replace the existing public key (${old_fpr}) with ${fpr}?" \
        || die "Aborted."
    fi
  fi

  gpg --export "${fpr}" > "${keys_dir}/skemer-release.gpg"
  gpg --armor --export "${fpr}" > "${keys_dir}/skemer-release.asc"
  printf '%s\n' "${fpr}" > "${keys_dir}/skemer-trusted"
  chmod 644 "${keys_dir}/skemer-release.gpg" \
    "${keys_dir}/skemer-release.asc" "${keys_dir}/skemer-trusted"

  local public_fpr
  public_fpr="$(gpg --batch --show-keys --with-colons "${keys_dir}/skemer-release.gpg" 2>/dev/null \
    | awk -F: '$1 == "fpr" { print $10; exit }')"
  [[ "${public_fpr}" == "${fpr}" ]] || die "Exported public key fingerprint does not match."

  local veldmuis_keys="${veldmuis_repo}/development/skemer-keyring"
  mkdir -p "${veldmuis_keys}"
  cp "${keys_dir}/skemer-release.gpg" "${veldmuis_keys}/skemer-release.gpg"
  cp "${keys_dir}/skemer-trusted" "${veldmuis_keys}/skemer-trusted"
  chmod 644 "${veldmuis_keys}/skemer-release.gpg" "${veldmuis_keys}/skemer-trusted"
  log "Public key files written to both checkouts."

  local subkey_file="${secrets_dir}/skemer-release-subkeys.asc"
  local full_file="${secrets_dir}/skemer-release-full.asc"
  (
    umask 077
    gpg --batch --armor --export-secret-subkeys "${fpr}" > "${subkey_file}"
    gpg --batch --armor --export-secret-keys "${fpr}" > "${full_file}"
  )
  chmod 600 "${subkey_file}" "${full_file}"
  [[ -s "${subkey_file}" ]] || die "The signing subkey export is empty."
  [[ -s "${full_file}" ]] || die "The full key export is empty."
  log "Secret files written to ${secrets_dir}:"
  log "  skemer-release-subkeys.asc  (GitHub signing key)"
  log "  skemer-release-full.asc     (offline full-key backup)"

  if ask_yes "Set the GitHub release environment secrets now?"; then
    if ! gh api "repos/${repo_slug}/environments/release" >/dev/null 2>&1; then
      log "Creating the release environment..."
      gh api --method PUT "repos/${repo_slug}/environments/release" >/dev/null \
        || die "Could not create the release environment; create it in repository settings."
    fi
    gh secret set SKEMER_GPG_PRIVATE_KEY --env release --repo "${repo_slug}" < "${subkey_file}"
    gh secret set SKEMER_GPG_FPR --env release --repo "${repo_slug}" --body "${fpr}"
    printf '%s' "${pass1}" \
      | gh secret set SKEMER_GPG_PASSPHRASE --env release --repo "${repo_slug}"
    log "GitHub secrets set:"
    gh secret list --env release --repo "${repo_slug}"
  else
    log "Skipped GitHub secrets. Set them manually:"
    log "  gh secret set SKEMER_GPG_PRIVATE_KEY --env release --repo ${repo_slug} < ${subkey_file}"
    log "  gh secret set SKEMER_GPG_FPR --env release --repo ${repo_slug} --body ${fpr}"
    log "  gh secret set SKEMER_GPG_PASSPHRASE --env release --repo ${repo_slug}"
  fi
  unset pass1

  log "Done. Review and commit the public files yourself:"
  printf '  git -C %q add packaging/keys/skemer-release.gpg packaging/keys/skemer-release.asc packaging/keys/skemer-trusted\n' "${skemer_repo}"
  printf '  git -C %q commit -m "chore: add the Skemer release public key"\n' "${skemer_repo}"
  printf '  git -C %q push\n' "${skemer_repo}"
  printf '  git -C %q add development/skemer-keyring/skemer-release.gpg development/skemer-keyring/skemer-trusted\n' "${veldmuis_repo}"
  printf '  git -C %q commit -m "chore(skemer): vendor the Skemer release public key"\n' "${veldmuis_repo}"
  printf '  git -C %q push\n' "${veldmuis_repo}"
  log "After merging both PRs, tag the first release from the main Skemer checkout:"
  log "  git -C ${HOME}/Projects/Skemer switch main"
  log "  git -C ${HOME}/Projects/Skemer pull --ff-only"
  log "  git -C ${HOME}/Projects/Skemer tag -a v0.1.0 -m 'Skemer 0.1.0'"
  log "  git -C ${HOME}/Projects/Skemer push origin v0.1.0"
  log "Then run the Veldmuis release watch:"
  log "  gh workflow run skemer-release-watch.yml --repo ruannnebornman/veldmuis"
  log "Keep ${secrets_dir} out of both repositories and store it in your vault."
}

main "$@"
