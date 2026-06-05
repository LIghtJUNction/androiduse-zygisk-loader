# AndroidUse Zygisk Loader

Rust Zygisk API v5 dynamic loader used by the AndroidUse module.

This repository is source infrastructure, not a standalone Magisk/KernelSU/APatch
module package. AndroidUse builds this crate into its own module ZIP and installs
the produced shared library under the AndroidUse module's `zygisk/` directory.

## Runtime Contract

The loader reads AndroidUse-owned runtime files:

```text
/data/adb/modules/AndroidUse/.config/androiduse/zygisk-target
/data/adb/modules/AndroidUse/.config/androiduse/payload.so
```

`zygisk-target` contains the target package/process name. When a matching app
specializes, the loader buffers `payload.so` while still in the Zygote phase,
writes it to the target app cache after specialization, loads it with `dlopen`,
then unlinks the temporary cache file.

## Build

Build for Android with `cargo-ndk` from the AndroidUse parent project:

```sh
cargo ndk -t arm64-v8a build --release
```

The AndroidUse build scripts are responsible for copying the resulting `.so`
into the module package.
