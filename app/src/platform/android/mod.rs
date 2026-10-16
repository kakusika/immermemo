//! Android entry point, platform keystore, the startup-splash handoff,
//! shared-storage access, system UI (status bar) visibility, and the font
//! fallbacks Slint cannot derive from the system on its own.

pub mod activity;
pub mod fonts;
pub mod keystore;
pub mod splash;
pub mod storage_access;
pub mod system_ui;

use std::sync::Arc;

use immermemo_core::{AppData, TokenStore};

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
    let start = crate::StartVault {
        first_run: base.join("notes"),
        open: None,
    };
    let app_data = AppData::new(base.clone());
    let vm_ptr = app.vm_as_ptr();
    let activity_ptr = app.activity_as_ptr();
    let vm_addr = vm_ptr as usize;
    let token_store_for: TokenStoreFactory = Box::new(move |vault_dir| {
        let path = app_data.token_path(vault_dir)?;
        // Safety: `vm_ptr` is `app.vm_as_ptr()`, valid for the process's
        // lifetime; this closure doesn't outlive it.
        let store = unsafe {
            keystore::AndroidKeystoreTokenStore::new(vm_addr as *mut std::ffi::c_void, path)
        }?;
        Ok(Arc::new(store) as Arc<dyn TokenStore>)
    });
    slint::android::init(app).expect("initialize the Android backend");
    fonts::repair_fallbacks();
    unsafe { activity::init(vm_ptr, activity_ptr) };
    crate::platform::widget::init_android_widget_bridge(&base);
    let (initial_note_path, initial_import_uri) = (
        crate::platform::widget::read_launch_note_path(),
        read_launch_import_uri(),
    );
    run(
        start,
        base,
        token_store_for,
        initial_note_path,
        initial_import_uri,
    )
    .expect("run the app");
}

/// Reads the deep-link import URI (`immermemo://...`) off `MainActivity`'s intent, if present.
pub fn read_launch_import_uri() -> Option<String> {
    activity::call(|env, activity| {
        let value = env
            .call_method(activity, "consumeImportUri", "()Ljava/lang/String;", &[])?
            .l()?;
        activity::optional_string(env, value)
    })
    .flatten()
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
