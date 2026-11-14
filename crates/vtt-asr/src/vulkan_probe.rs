//! Pilote Vulkan présent ? (Linux)
//!
//! Compilé avec Vulkan, whisper.cpp initialise Vulkan dès le chargement,
//! même sans GPU demandé, et lève une exception C++ qui arrête le programme
//! si aucun pilote n'est installé (`vk::IncompatibleDriverError` : machine
//! virtuelle, carte NVIDIA sans pilote…). L'API C de Vulkan, elle, renvoie
//! un code d'erreur : on crée une instance de test avant de laisser
//! whisper.cpp le faire.

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

#[repr(C)]
struct ApplicationInfo {
    s_type: i32,
    p_next: *const c_void,
    application_name: *const c_char,
    application_version: u32,
    engine_name: *const c_char,
    engine_version: u32,
    api_version: u32,
}

#[repr(C)]
struct InstanceCreateInfo {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    application_info: *const ApplicationInfo,
    enabled_layer_count: u32,
    enabled_layer_names: *const *const c_char,
    enabled_extension_count: u32,
    enabled_extension_names: *const *const c_char,
}

type CreateInstance =
    unsafe extern "C" fn(*const InstanceCreateInfo, *const c_void, *mut *mut c_void) -> i32;
type DestroyInstance = unsafe extern "C" fn(*mut c_void, *const c_void);

const STRUCTURE_TYPE_APPLICATION_INFO: i32 = 0;
const STRUCTURE_TYPE_INSTANCE_CREATE_INFO: i32 = 1;
/// Vulkan 1.2, la version demandée par whisper.cpp.
const API_VERSION_1_2: u32 = (1 << 22) | (2 << 12);

/// `Err` avec la raison si Vulkan est inutilisable sur cet ordinateur.
/// Testé une fois par lancement.
pub fn driver_available() -> Result<(), String> {
    static RESULT: OnceLock<Result<(), String>> = OnceLock::new();
    RESULT.get_or_init(probe).clone()
}

fn probe() -> Result<(), String> {
    // SAFETY: bibliothèque système standard ; les signatures sont celles de
    // l'API C de Vulkan 1.0 (vkCreateInstance, vkDestroyInstance), les
    // structures sont initialisées en entier et vivent pendant l'appel.
    unsafe {
        let library = libloading::Library::new("libvulkan.so.1")
            .map_err(|e| format!("bibliothèque Vulkan absente : {e}"))?;
        let create: libloading::Symbol<CreateInstance> = library
            .get(b"vkCreateInstance\0")
            .map_err(|e| format!("vkCreateInstance : {e}"))?;
        let destroy: libloading::Symbol<DestroyInstance> = library
            .get(b"vkDestroyInstance\0")
            .map_err(|e| format!("vkDestroyInstance : {e}"))?;
        let app = ApplicationInfo {
            s_type: STRUCTURE_TYPE_APPLICATION_INFO,
            p_next: std::ptr::null(),
            application_name: c"VoiceToText".as_ptr(),
            application_version: 0,
            engine_name: std::ptr::null(),
            engine_version: 0,
            api_version: API_VERSION_1_2,
        };
        let info = InstanceCreateInfo {
            s_type: STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: 0,
            application_info: &app,
            enabled_layer_count: 0,
            enabled_layer_names: std::ptr::null(),
            enabled_extension_count: 0,
            enabled_extension_names: std::ptr::null(),
        };
        let mut instance: *mut c_void = std::ptr::null_mut();
        let code = create(&info, std::ptr::null(), &mut instance);
        if code != 0 {
            // -9 : VK_ERROR_INCOMPATIBLE_DRIVER (aucun pilote).
            return Err(format!("aucun pilote Vulkan utilisable (code {code})"));
        }
        destroy(instance, std::ptr::null());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Ne plante jamais, avec ou sans pilote (le serveur de développement
    /// n'en a pas).
    #[test]
    fn probe_never_crashes() {
        let first = super::driver_available();
        assert_eq!(super::driver_available(), first);
    }
}
