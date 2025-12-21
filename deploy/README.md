Secluso deploy tool (developer notes)

This repo is the full deploy workflow for Secluso. It builds a Raspberry Pi image, provisions a server over SSH, and shows live status in the UI while Docker or SSH steps run. Binaries are installed via secluso-update, and signature checks are optional but supported.

The UI lives in src/ with the SvelteKit pages, while src-tauri/ holds the backend commands and the shell scripts used for the image build and server provision. The image builder scripts live under src-tauri/assets/pi_hub/, the server install script lives under src-tauri/assets/server/, and the test/ folder has the manual harness and fixtures.

For dev work you need node 18+, pnpm, rust 1.85.0, Docker Desktop, and the normal Tauri system deps for your OS. Install and run dev with:
```
pnpm install
pnpm dev
pnpm tauri dev
```

Checks and production builds are:
```
pnpm check
pnpm build
pnpm tauri build
```

There are two main flows. The image build flow collects output paths and optional dev settings, generates the camera secret through secluso-update, then builds the image in Docker and prefetches the hub binary. The server flow collects the SSH target plus credentials, generates user credentials with secluso-update, then runs the remote install script and enables services.

Updater wise, we bootstrap secluso-update by building it from the update submodule pinned to the latest release tag. That bootstrap updater fetches the server or hub binary before any service starts. After that, we install the bundled secluso-update from the release zip so runtime updates use the distributed binary for the target arch.

Developer settings are stored in localStorage under secluso-dev-settings. Developer mode lets you set a Wi‑Fi preset for first boot and a custom repo plus signature keys for updater verification. Signature keys are passed as name:github_user via --sig-key.

