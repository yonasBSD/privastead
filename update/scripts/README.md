# Secluso release scripts — simple + real-world

These scripts build the zip you upload to GitHub Releases:

- secluso-vX.Y.Z.zip

That zip contains:
- manifest.json (signed bytes)
- manifest.json.<labelA>.asc and manifest.json.<labelB>.asc (2 detached sigs over manifest.json)
- binaries in arch folders like:
- aarch64-unknown-linux-gnu/...
- x86_64-unknown-linux-gnu/...

Updater does as follows...
- downloads the zip
- verifies both OpenPGP signatures over manifest.json
- picks one binary (based on input), checks its sha256 against the signed manifest
- installs that one binary into the designated root subdirectory

---

## Real release (2 people sign, manual upload)

### Step 0 - Build artifacts (Docker builder)

Should end up with a folder like:

- /path/to/builder_out/
- manifest.json
- aarch64-unknown-linux-gnu/...
- x86_64-unknown-linux-gnu/...

This whole folder is your artifact dir (passed as --artifact-dir).

### Step 1 - Manager preps the manifest to sign

We just copy builder manifest.json into the release_work folder and write a manifest.sha256 so signers can sanity check they are signing the right bytes.

Run:
- ./secluso_prepare_release_dir.sh --tag v0.4.0 --workdir ./release_work --labels jkaczman,arrdalan --artifact-dir /path/to/builder_out

This creates:
- ./release_work/v0.4.0/manifest.json (copied from builder_out/manifest.json, unchanged)
- ./release_work/v0.4.0/manifest.sha256 (sha256 of manifest.json)

Send both files to both signers:
- ./release_work/v0.4.0/manifest.json
- ./release_work/v0.4.0/manifest.sha256

### Step 2 - Each signer signs it

Signer A runs:
- ./secluso_sign_manifest.sh --manifest ./release_work/v0.4.0/manifest.json --sha-file ./release_work/v0.4.0/manifest.sha256 --label jkaczman --key <YOUR_KEY_FPR> --outdir ./release_work/v0.4.0/sigs

Signer B runs:
- ./secluso_sign_manifest.sh --manifest ./release_work/v0.4.0/manifest.json --sha-file ./release_work/v0.4.0/manifest.sha256 --label arrdalan --key <YOUR_KEY_FPR> --outdir ./release_work/v0.4.0/sigs

They send back the .asc files:
- manifest.json.jkaczman.asc
- manifest.json.arrdalan.asc

### Step 3 - Manager builds the release zip to bundle everything

Run:
- ./secluso_build_bundle.sh --tag v0.4.0 --workdir ./release_work --labels jkaczman,arrdalan --sig-a ./release_work/v0.4.0/sigs/manifest.json.jkaczman.asc --sig-b ./release_work/v0.4.0/sigs/manifest.json.arrdalan.asc --artifact-dir /path/to/builder_out

Output zip:
- ./release_work/v0.4.0/out/secluso-v0.4.0.zip

### Step 4 - Upload manually

- GitHub -> Releases -> Draft new release
- tag v0.4.0
- upload secluso-v0.4.0.zip
- publish

---

## Test release (solo, for testing)

This makes a reproducible build style bundle (arch folders + multiple binaries), but uses dummy binaries for easy testing

Run:
- ./secluso_make_test_bundle.sh --tag v0.0.1 --workdir /tmp/release_work --labels jkaczman-1,jkaczman-2

Output zip:
- /tmp/release_work/v0.0.1/out/secluso-v0.0.1.zip

### Verify test solo

The test script uses 2 test keys (and can reuse them if GNUPGHOME stays the same). Export both public keys and add them to GitHub as GPG keys.

1) Set GNUPGHOME to whatever the script printed:
- export GNUPGHOME=/tmp/secluso_test_gnupg

2) List keys and get fingerprints:
- gpg --list-keys --fingerprint

3) Export both keys using the fingerprints you saw:
- gpg --armor --export <FPR1> > /tmp/testkey1.pub.asc
- gpg --armor --export <FPR2> > /tmp/testkey2.pub.asc

4) Add to GitHub:
- GitHub Settings -> SSH and GPG keys -> New GPG key
- paste contents of each .pub.asc