//! Android entry point, platform keystore, and certificate verification.

pub mod cert;
pub mod keystore;

use std::sync::Arc;

use immermemo_vault::appdata::AppData;
use immermemo_vault::credentials::TokenStore;

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
    unsafe {
        haptic::init_android_haptics(vm_ptr, activity_ptr);
    }
    run(default_vault_dir, base, token_store_for).expect("run the app");
}
