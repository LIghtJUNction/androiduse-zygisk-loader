use std::os::raw::*;

use jni::{objects::JString, sys::*};

#[allow(non_camel_case_types)]
type c_bool = bool;
type Module = crate::module::RawModule;

pub const API_VERSION: c_long = 5;

#[repr(C)]
pub(crate) struct ModuleAbi {
    pub api_version: c_long,
    pub this: &'static mut Module,
    pub pre_app_specialize: extern "C" fn(&mut Module, &mut AppSpecializeArgs),
    pub post_app_specialize: extern "C" fn(&mut Module, &AppSpecializeArgs),
    pub pre_server_specialize: extern "C" fn(&mut Module, &mut ServerSpecializeArgs),
    pub post_server_specialize: extern "C" fn(&mut Module, &ServerSpecializeArgs),
}

#[repr(C)]
pub(crate) struct RawApiTable {
    // These first 2 entries are permanent, shall never change across API versions
    pub this: *const (),
    pub register_module: Option<extern "C" fn(*const RawApiTable, *mut ModuleAbi) -> c_bool>,

    // Utility functions
    pub hook_jni_native_methods:
        Option<extern "C" fn(*mut JNIEnv, *const c_char, *mut JNINativeMethod, c_int)>,

    // PLT Hook functions - API version dependent
    pub plt_hook_register:
        Option<extern "C" fn(*const c_char, *const c_char, *mut (), *mut *mut ())>, // v3 and below
    pub plt_hook_exclude: Option<extern "C" fn(*const c_char, *const c_char)>, // v3 and below
    pub plt_hook_commit: Option<extern "C" fn() -> c_bool>,

    // Zygisk functions
    pub connect_companion: Option<extern "C" fn(*const ()) -> c_int>,
    pub set_option: Option<extern "C" fn(*const (), ZygiskOption)>,
    pub get_module_dir: Option<extern "C" fn(*const ()) -> c_int>,
    pub get_flags: Option<extern "C" fn(*const ()) -> u32>,

    // API v4+ functions
    pub plt_hook_register_v4:
        Option<extern "C" fn(c_ulong, c_ulong, *const c_char, *mut (), *mut *mut ())>, // dev_t, ino_t variant
    pub exempt_fd: Option<extern "C" fn(c_int)>, // v4+ replacement for plt_hook_exclude
}

#[repr(C)]
pub struct AppSpecializeArgs<'a> {
    // Required arguments. These arguments are guaranteed to exist on all Android versions.
    pub uid: &'a mut jint,
    pub gid: &'a mut jint,
    pub gids: &'a mut jintArray,
    pub runtime_flags: &'a mut jint,
    pub rlimits: Option<&'a jobjectArray>, // API v4+
    pub mount_external: &'a mut jint,
    pub se_info: &'a mut JString<'a>,
    pub nice_name: &'a mut JString<'a>,
    pub instruction_set: &'a mut JString<'a>,
    pub app_data_dir: &'a mut JString<'a>,

    // Optional arguments. Please check whether the pointer is null before de-referencing
    pub fds_to_ignore: Option<&'a jintArray>, // API v4+
    pub is_child_zygote: Option<&'a jboolean>,
    pub is_top_app: Option<&'a jboolean>,
    pub pkg_data_info_list: Option<&'a jobjectArray>,
    pub whitelisted_data_info_list: Option<&'a jobjectArray>,
    pub mount_data_dirs: Option<&'a jboolean>,
    pub mount_storage_dirs: Option<&'a jboolean>,
    pub mount_sysprop_overrides: Option<&'a jboolean>, // API v5
}

#[repr(C)]
pub struct ServerSpecializeArgs<'a> {
    pub uid: &'a mut jint,
    pub gid: &'a mut jint,
    pub gids: &'a mut jintArray,
    pub runtime_flags: &'a mut jint,
    pub permitted_capabilities: &'a mut jlong,
    pub effective_capabilities: &'a mut jlong,
}

/// Zygisk module options, used in [ZygiskApi::set_option()](crate::ZygiskApi::set_option).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZygiskOption {
    /// Force ReZygisk to unmount the root related mounts on this process.
    /// This option will only take effect if set in pre...Specialize, as ReZygisk unmounts at that point.
    ///
    /// ReZygisk Unmount System will not unmount all root related mounts, read ReZygiskd
    /// unmount_root function in utils.c file to understand how it selects the ones to unmount.
    ///
    /// Setting this option only makes sense in `preAppSpecialize`.
    /// The actual unmounting happens during app process specialization.
    ///
    /// Set this option to force all root manager and modules' files to be unmounted from the
    /// mount namespace of the process, regardless of the denylist enforcement status.
    ForceDenylistUnmount = 0,

    /// Once set, ReZygisk will dlclose your library from the process, this is assured to
    /// happen after post...Specialize, but not at a specific moment due to different implementations.
    ///
    /// You should not use this option if you leave references in the process such as hooks,
    /// which will try to execute uninitialized memory.
    ///
    /// When this option is set, your module's library will be `dlclose`-ed after `post[XXX]Specialize`.
    /// Be aware that after `dlclose`-ing your module, all of your code will be unmapped from memory.
    ///
    /// YOU MUST NOT ENABLE THIS OPTION AFTER HOOKING ANY FUNCTIONS IN THE PROCESS.
    DlcloseModuleLibrary = 1,
}

bitflags::bitflags! {
    /// Bit masks of the return value of [ZygiskApi::get_flags()](crate::ZygiskApi::get_flags).
    pub struct StateFlags: u32 {
        /// The user has granted root access to the current process.
        const PROCESS_GRANTED_ROOT = (1 << 0);

        /// The current process was added on the denylist.
        const PROCESS_ON_DENYLIST = (1 << 1);

        /// The current process is a manager process.
        const PROCESS_IS_MANAGER = (1 << 27);

        /// The root implementation is APatch.
        const PROCESS_ROOT_IS_APATCH = (1 << 28);

        /// The root implementation is KernelSU.
        const PROCESS_ROOT_IS_KSU = (1 << 29);

        /// The root implementation is Magisk.
        const PROCESS_ROOT_IS_MAGISK = (1 << 30);

        /// This is the first time the process is started.
        const PROCESS_IS_FIRST_STARTED = (1 << 31);
    }
}
