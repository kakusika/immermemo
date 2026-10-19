//! Android platform entry point and JNI callbacks.

use std::sync::Arc;

use immermemo_core::{AppData, TokenStore};
use immermemo_platform::android::keystore::AndroidKeystoreTokenStore;
use immermemo_platform::android::{activity, fonts, read_launch_import_uri};
use immermemo_platform::widget;
use immermemo_shell::{StartVault, TokenStoreFactory, handle_incoming_import_uri, run};

/// Android starts the app here (the activity loads this library), not at
/// `main`. Everything -- notes, the private gitdir, the encrypted token --
/// lives under the app's private storage; Android has no `HOME` or
/// `XDG_DATA_HOME` for the desktop paths to fall back on.
#[unsafe(no_mangle)]
pub fn android_main(app: slint::android::AndroidApp) {
    let base = app
        .internal_data_path()
        .expect("the app has private storage");
    let start = StartVault {
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
            AndroidKeystoreTokenStore::new(vm_addr as *mut std::ffi::c_void, path)
        }?;
        Ok(Arc::new(store) as Arc<dyn TokenStore>)
    });
    slint::android::init(app).expect("initialize the Android backend");
    fonts::repair_fallbacks();
    unsafe { activity::init(vm_ptr, activity_ptr) };
    widget::init_android_widget_bridge(&base);
    let (initial_note_path, initial_import_uri) = (
        widget::read_launch_note_path(),
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
            handle_incoming_import_uri(&uri_str);
        });
    }
}

/// Called from `MainActivity.onActivityResult` with the tree URI the folder
/// picker returned.
#[unsafe(no_mangle)]
pub extern "C" fn Java_dev_immermemo_app_MainActivity_notifyFolderPicked(
    mut env: jni::JNIEnv,
    _activity: jni::objects::JObject,
    uri: jni::objects::JString,
) {
    if let Ok(uri) = env.get_string(&uri) {
        let uri: String = uri.into();
        let _ = slint::invoke_from_event_loop(move || {
            immermemo_shell::callbacks::vault_setup::folder_picked(&uri);
        });
    }
}
