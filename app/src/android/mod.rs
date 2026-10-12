//! Android entry point, platform keystore, the startup-splash handoff,
//! and system UI (status bar) visibility.

pub mod keystore;
pub mod splash;
pub mod system_ui;

use std::sync::Arc;

use immermemo_core::{AppData, TokenStore};

use crate::haptic;
use crate::{TokenStoreFactory, run};

/// Android starts the app here (the activity loads this library), not at
/// `main`. Everything -- notes, the private gitdir, the encrypted token --
/// lives under the app's private storage; Android has no `HOME` or
/// `XDG_DATA_HOME` for the desktop paths to fall back on.
#[unsafe(no_mangle)]
pub fn android_main(app: slint::android::AndroidApp) {
    let base = app
        .internal_data_path()
        .expect("the app has private storage");
    let default_vault_dir = base.join("notes");
    let app_data = AppData::new(base.clone());
    let vm_ptr = app.vm_as_ptr();
    let activity_ptr = app.activity_as_ptr();
    let token_store_for: TokenStoreFactory = Box::new(move |vault_dir| {
        let path = app_data.token_path(vault_dir)?;
        // Safety: `vm_ptr` is `app.vm_as_ptr()`, valid for the process's
        // lifetime; this closure doesn't outlive it.
        let store = unsafe { keystore::AndroidKeystoreTokenStore::new(vm_ptr, path) }?;
        Ok(Arc::new(store) as Arc<dyn TokenStore>)
    });
    slint::android::init(app).expect("initialize the Android backend");
    let (initial_note_path, initial_import_uri) = unsafe {
        haptic::init_android_haptics(vm_ptr, activity_ptr);
        splash::init(vm_ptr, activity_ptr);
        system_ui::init(vm_ptr, activity_ptr);
        crate::widget::init_android_widget_bridge(vm_ptr, activity_ptr, &base);
        (
            crate::widget::read_launch_note_path(vm_ptr, activity_ptr),
            read_launch_import_uri(vm_ptr, activity_ptr),
        )
    };
    run(
        default_vault_dir,
        base,
        token_store_for,
        initial_note_path,
        initial_import_uri,
    )
    .expect("run the app");
}

/// Reads the deep-link import URI (`immermemo://...`) off `MainActivity`'s intent, if present.
pub unsafe fn read_launch_import_uri(
    vm_ptr: *mut std::ffi::c_void,
    activity_ptr: *mut std::ffi::c_void,
) -> Option<String> {
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr.cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { jni::objects::JObject::from_raw(activity_ptr.cast()) };
    let result: jni::errors::Result<Option<String>> = (|| {
        let value = env
            .call_method(&activity, "consumeImportUri", "()Ljava/lang/String;", &[])?
            .l()?;
        if value.is_null() {
            return Ok(None);
        }
        let jstring = jni::objects::JString::from(value);
        Ok(Some(env.get_string(&jstring)?.into()))
    })();
    result.ok().flatten()
}

/// Called from `MainActivity.onNewIntent` when the user opens a deep-link while the app is running.
#[unsafe(no_mangle)]
pub extern "C" fn Java_dev_immermemo_app_MainActivity_notifyDeepLink(
    mut env: jni::JNIEnv,
    _activity: jni::objects::JObject,
    uri: jni::objects::JString,
) {
    if let Ok(uri_jstr) = env.get_string(&uri) {
        let uri_str: String = uri_jstr.into();
        let _ = slint::invoke_from_event_loop(move || {
            crate::handle_incoming_import_uri(&uri_str);
        });
    }
}
