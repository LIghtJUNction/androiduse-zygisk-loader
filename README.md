<div align="center">
  <h1>AndroidUse Zygisk Loader</h1>
  <p><strong>A small Rust Zygisk API v5 loader for AndroidUse AUZM payloads.</strong></p>
  <p>registry discovery · process scopes · pre-specialization buffering · memfd loading</p>

  [![Version](https://img.shields.io/badge/version-0.1.0-111111?style=flat-square)](Cargo.toml)
  [![Rust](https://img.shields.io/badge/Rust-2021-111111?style=flat-square&logo=rust)](Cargo.toml)
  [![Android](https://img.shields.io/badge/target-Android%20arm64--v8a-111111?style=flat-square&logo=android)](#build)
  [![License](https://img.shields.io/badge/license-MIT-111111?style=flat-square)](LICENSE)

  [Lifecycle](#runtime-lifecycle) · [Registry](#auzm-registry-contract) · [Scopes](#scope-matching) · [Build](#build)
</div>

> [!IMPORTANT]
> This repository is source infrastructure, not a standalone Magisk, KernelSU, APatch, or AndroidUse module ZIP. The AndroidUse parent project builds this crate and places the resulting shared library in its own `zygisk/` directory.

## What it does

The loader has one narrow job: choose enabled AndroidUse Zygisk Modules (AUZM) for the process currently being specialized, preserve their bytes across the Zygote transition, then load them into that child process.

```text
AndroidUse AUZM registry
          ↓
read enabled entry + scope + payload path
          ↓
match current app/process during preAppSpecialize
          ↓
read matching payloads into memory
          ↓
app specialization completes
          ↓
create anonymous memfd + android_dlopen_ext
```

The current implementation does **not** copy payloads into an app cache directory. It writes each buffered payload into an anonymous `memfd`, loads it with `android_dlopen_ext`, then closes the file descriptor.

## Runtime lifecycle

### 1. `on_load`

The Zygisk runtime supplies JNI state. The loader stores the `JavaVM` reference and initializes Android logging under the `AndroidUseZygisk` tag.

### 2. `pre_app_specialize`

While still in the Zygote specialization path, the loader:

1. resolves the current process name from `nice_name`, `/proc/self/cmdline`, or the app data path;
2. reads enabled entries from the AUZM registry;
3. applies each entry's scope rules;
4. reads every matching `.so` completely into memory.

Reading before specialization matters because module-owned paths available in the Zygote context may no longer be reachable under the child process's final mount and permission state.

### 3. `post_app_specialize`

For a matched process, the loader:

- exports `ANDROIDUSE_PROCESS_NAME`;
- exports `ANDROIDUSE_APP_DATA_DIR`;
- creates one anonymous memfd per payload;
- writes and rewinds the payload bytes;
- loads through `android_dlopen_ext` with `ANDROID_DLEXT_USE_LIBRARY_FD | ANDROID_DLEXT_FORCE_LOAD`;
- closes the descriptor after the load call.

A failed payload is logged and skipped; it does not cause the loader to claim success.

## AUZM registry contract

Primary registry:

```text
/data/adb/modules/AndroidUse/.config/androiduse/auzm.d/<module-id>/
```

Recognized entry files:

```text
enabled
name
scope
path
payload
payload.so
```

The loader currently uses:

| File | Meaning |
| --- | --- |
| `enabled` | Optional boolean-like state; missing or empty currently counts as enabled |
| `scope` | Required newline-separated process match rules |
| `path` | Preferred absolute payload path |
| `payload` | Compatibility alternative to `path` |
| `payload.so` | Default payload path inside the entry directory |
| `name` | Registry metadata owned by AndroidUse; not required by this loader path |

Payload path resolution is:

```text
path → payload → <entry>/payload.so
```

Entries with no usable payload path or empty scope are ignored.

Accepted enabled values are empty text, `1`, `true`, `yes`, `on`, `enabled`, and `active`. Other values disable the entry.

## Legacy fallback

If the registry contains no usable entries, the loader falls back to the older AndroidUse files:

```text
/data/adb/modules/AndroidUse/.config/androiduse/zygisk-target
/data/adb/modules/AndroidUse/.config/androiduse/payload.so
```

The first line of `zygisk-target` becomes the scope for a synthetic `androiduse-runtime` entry. This path exists for compatibility with older installs; new AndroidUse packaging should use `auzm.d/`.

## Scope matching

`scope` is parsed line by line:

```text
# comments are ignored
com.example.app
com.example.app:remote
*
```

A rule matches when:

```pseudo
rule == "*" OR process_name.contains(rule)
```

This is substring matching, not glob syntax, package-manager validation, or an exact-only policy. A broad token can match more processes than expected. Registry generation should therefore use full package/process names whenever possible.

The scope file controls **where AndroidUse attempts to load a payload**. It is not a security boundary against a malicious process that can already modify `/data/adb/modules/AndroidUse`.

## Payload interface

The loader sets two environment variables before loading matched payloads:

```text
ANDROIDUSE_PROCESS_NAME
ANDROIDUSE_APP_DATA_DIR
```

A payload may read them during initialization. Their presence does not guarantee that every process has a normal application data directory; if the loader cannot resolve that directory, post-specialization loading stops for that process.

Payloads are native code executing inside the target process. They inherit that process's failure surface and can crash or alter it. Only load payloads whose source and build provenance you trust.

## Build

Build from the AndroidUse parent project with an installed Android NDK and `cargo-ndk`:

```bash
cargo ndk -t arm64-v8a build --release
```

The crate produces a `cdylib`. AndroidUse's parent build is responsible for copying and naming the resulting `.so` inside the final module ZIP.

Release settings favor a small injected library:

```text
opt-level = z
LTO = enabled
codegen-units = 1
symbols = stripped
panic = abort
```

A host `cargo check` can validate portable Rust code, but it does not prove Android linking, Zygisk ABI compatibility, JNI argument handling, or payload loading on a device.

## Repository map

```text
src/api.rs       safe wrapper around the Zygisk API table
src/binding.rs   API v5 ABI bindings and specialize arguments
src/macros.rs    exported module/companion entry macros
src/module.rs    Zygisk module trait and registration bridge
src/lib.rs       AndroidUse registry, matching, buffering and memfd loading
```

## Operational limits

- Only Android app specialization is used; this README does not claim server-process loading.
- The current registry scan has no cryptographic signature or payload hash verification.
- Scope rules use substring matching.
- A missing `enabled` file currently defaults to enabled.
- Payloads are buffered without a repository-level size limit.
- The only in-crate test is a compile-time companion registration smoke path; real device integration remains the meaningful acceptance test.

These are implementation facts, not guarantees supplied by the README.

## License

[MIT](LICENSE)
