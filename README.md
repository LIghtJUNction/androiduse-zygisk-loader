# AndroidUse Zygisk Loader

Rust Zygisk API v5 dynamic loader used by the AndroidUse module.

This repository is source infrastructure, not a standalone Magisk/KernelSU/APatch
module package. AndroidUse builds this crate into its own module ZIP and installs
the produced shared library under the AndroidUse module's `zygisk/` directory.

## Runtime Contract

The loader reads AndroidUse-owned runtime files:

```text
/data/adb/modules/AndroidUse/.config/androiduse/zygisk-target
/data/adb/modules/AndroidUse/.config/androiduse/auzm.d/<module-id>/
```

Each AUZM registry directory contains small text files:

```text
enabled
name
scope
path
payload.so
```

`scope` contains package/process match strings, one per line. When a matching
app specializes, the loader buffers every enabled AUZM whose scope matches while
still in the Zygote phase, writes each `.so` to the target app cache after
specialization, loads it with `dlopen`, then unlinks the temporary cache file.
`zygisk-target` is preserved as compatibility config for older installs and the
default runtime module.

## Build

Build for Android with `cargo-ndk` from the AndroidUse parent project:

```sh
cargo ndk -t arm64-v8a build --release
```

The AndroidUse build scripts are responsible for copying the resulting `.so`
into the module package.
