//! Data models, encoding, decoding, and application logic for transferring
//! application settings and credentials across devices (via QR code or deep links).

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::appdata::AppData;
use crate::credentials::TokenStore;

pub const SCHEME_PREFIX: &str = "immermemo://import#";
pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const APP_IDENTIFIER: &str = "immermemo";

/// Options specifying which settings categories to include when exporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportOptions {
    pub include_interface: bool,
    pub include_remote: bool,
    pub include_token: bool,
}

/// The top-level export payload encoded into QR codes and transfer links.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferPayload {
    pub v: u32,
    pub app: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<InterfaceTransfer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncTransfer>,
}

/// Appearance and UI preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct InterfaceTransfer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_bar: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_scale: Option<f32>,
}

/// Remote vault synchronization settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SyncTransfer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Summary of what was modified when applying an imported payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApplyResult {
    pub interface_updated: bool,
    pub remote_updated: bool,
    pub token_updated: bool,
}

/// Builds a [`TransferPayload`] from current settings based on [`ExportOptions`].
pub fn build_payload(
    app_data: &AppData,
    vault_dir: &Path,
    token_store: &dyn TokenStore,
    options: &ExportOptions,
    vault_name: Option<String>,
    ui_scale: Option<f32>,
) -> anyhow::Result<TransferPayload> {
    let interface = if options.include_interface {
        Some(InterfaceTransfer {
            theme: app_data.load_theme(),
            font_size: app_data.load_font_size(),
            status_bar: app_data.load_status_bar_choice(),
            ui_scale,
        })
    } else {
        None
    };

    let sync = if options.include_remote || options.include_token {
        let remote_url = if options.include_remote {
            app_data.load_remote(vault_dir)
        } else {
            None
        };
        let token = if options.include_token {
            token_store.load()?
        } else {
            None
        };
        Some(SyncTransfer {
            vault_name,
            remote_url,
            token,
        })
    } else {
        None
    };

    Ok(TransferPayload {
        v: CURRENT_SCHEMA_VERSION,
        app: APP_IDENTIFIER.to_owned(),
        interface,
        sync,
    })
}

/// Serializes and Base64-encodes a [`TransferPayload`] into an `immermemo://import#...` URI.
pub fn encode_payload(payload: &TransferPayload) -> anyhow::Result<String> {
    let json = serde_json::to_string(payload)?;
    let b64 = URL_SAFE_NO_PAD.encode(json.as_bytes());
    Ok(format!("{SCHEME_PREFIX}{b64}"))
}

/// Decodes and validates a raw transfer string (URI scheme, base64 blob, or plain JSON).
pub fn decode_payload(raw: &str) -> anyhow::Result<TransferPayload> {
    let trimmed = raw.trim();
    let content = if let Some(stripped) = trimmed.strip_prefix(SCHEME_PREFIX) {
        stripped
    } else if let Some(stripped) = trimmed.strip_prefix("immermemo://import?data=") {
        stripped
    } else {
        trimmed
    };

    // Try decoding as URL-safe base64, then standard base64, then raw json.
    let json_bytes = URL_SAFE_NO_PAD
        .decode(content)
        .or_else(|_| STANDARD.decode(content))
        .unwrap_or_else(|_| content.as_bytes().to_vec());

    let payload: TransferPayload = serde_json::from_slice(&json_bytes)
        .map_err(|e| anyhow::anyhow!("invalid payload format: {e}"))?;

    if payload.app != APP_IDENTIFIER {
        anyhow::bail!("unsupported application identifier: {}", payload.app);
    }
    if payload.v != CURRENT_SCHEMA_VERSION {
        anyhow::bail!("unsupported schema version: {}", payload.v);
    }

    Ok(payload)
}

/// Applies imported settings from a [`TransferPayload`] to the given [`AppData`] and [`TokenStore`].
pub fn apply_payload(
    app_data: &AppData,
    vault_dir: &Path,
    token_store: &dyn TokenStore,
    payload: &TransferPayload,
) -> anyhow::Result<ApplyResult> {
    let mut result = ApplyResult::default();

    if let Some(interface) = &payload.interface {
        if let Some(theme) = interface.theme {
            app_data.save_theme(theme)?;
            result.interface_updated = true;
        }
        if let Some(font_size) = interface.font_size {
            app_data.save_font_size(font_size)?;
            result.interface_updated = true;
        }
        if let Some(status_bar) = interface.status_bar {
            app_data.save_status_bar_choice(status_bar)?;
            result.interface_updated = true;
        }
    }

    if let Some(sync) = &payload.sync {
        if let Some(url) = &sync.remote_url {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                app_data.save_remote(vault_dir, trimmed)?;
                result.remote_updated = true;
            }
        }
        if let Some(token) = &sync.token {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                token_store.save(trimmed)?;
                result.token_updated = true;
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::PlainFileTokenStore;

    #[test]
    fn round_trip_encoding_and_decoding() {
        let payload = TransferPayload {
            v: 1,
            app: "immermemo".to_owned(),
            interface: Some(InterfaceTransfer {
                theme: Some(2),
                font_size: Some(0),
                status_bar: Some(1),
                ui_scale: Some(1.1),
            }),
            sync: Some(SyncTransfer {
                vault_name: Some("Personal".to_owned()),
                remote_url: Some("https://github.com/test/notes.git".to_owned()),
                token: Some("ghp_secret123".to_owned()),
            }),
        };

        let uri = encode_payload(&payload).unwrap();
        assert!(uri.starts_with(SCHEME_PREFIX));

        let decoded = decode_payload(&uri).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn decode_handles_raw_json() {
        let json = r#"{"v":1,"app":"immermemo","interface":{"theme":1}}"#;
        let decoded = decode_payload(json).unwrap();
        assert_eq!(decoded.v, 1);
        assert_eq!(decoded.interface.unwrap().theme, Some(1));
    }

    #[test]
    fn decode_rejects_foreign_app() {
        let json = r#"{"v":1,"app":"otherapp","interface":{"theme":1}}"#;
        assert!(decode_payload(json).is_err());
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let json = r#"{"v":99,"app":"immermemo","interface":{"theme":1}}"#;
        assert!(decode_payload(json).is_err());
    }

    #[test]
    fn build_and_apply_payload() {
        let dir = tempfile::tempdir().unwrap();
        let app_data = AppData::new(dir.path().join("appdata"));
        let vault_dir = dir.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();

        let token_path = app_data.token_path(&vault_dir).unwrap();
        let token_store = PlainFileTokenStore::new(token_path);

        // Pre-configure some settings
        app_data.save_theme(2).unwrap();
        app_data.save_font_size(1).unwrap();
        app_data
            .save_remote(&vault_dir, "https://github.com/initial/repo.git")
            .unwrap();
        token_store.save("ghp_old_token").unwrap();

        // Build with token included
        let options = ExportOptions {
            include_interface: true,
            include_remote: true,
            include_token: true,
        };
        let payload = build_payload(
            &app_data,
            &vault_dir,
            &token_store,
            &options,
            Some("My Vault".to_owned()),
            Some(1.25),
        )
        .unwrap();

        assert_eq!(payload.interface.as_ref().unwrap().theme, Some(2));
        assert_eq!(
            payload.sync.as_ref().unwrap().remote_url,
            Some("https://github.com/initial/repo.git".to_owned())
        );
        assert_eq!(
            payload.sync.as_ref().unwrap().token,
            Some("ghp_old_token".to_owned())
        );

        // Apply on a fresh vault
        let dir2 = tempfile::tempdir().unwrap();
        let app_data2 = AppData::new(dir2.path().join("appdata"));
        let vault_dir2 = dir2.path().join("vault");
        std::fs::create_dir_all(&vault_dir2).unwrap();
        let token_path2 = app_data2.token_path(&vault_dir2).unwrap();
        let token_store2 = PlainFileTokenStore::new(token_path2);

        let result = apply_payload(&app_data2, &vault_dir2, &token_store2, &payload).unwrap();
        assert!(result.interface_updated);
        assert!(result.remote_updated);
        assert!(result.token_updated);

        assert_eq!(app_data2.load_theme(), Some(2));
        assert_eq!(
            app_data2.load_remote(&vault_dir2),
            Some("https://github.com/initial/repo.git".to_owned())
        );
        assert_eq!(
            token_store2.load().unwrap(),
            Some("ghp_old_token".to_owned())
        );
    }

    #[test]
    fn token_excluded_when_not_selected() {
        let dir = tempfile::tempdir().unwrap();
        let app_data = AppData::new(dir.path().join("appdata"));
        let vault_dir = dir.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        let token_path = app_data.token_path(&vault_dir).unwrap();

        let token_store = PlainFileTokenStore::new(token_path);
        token_store.save("ghp_secret").unwrap();

        let options = ExportOptions {
            include_interface: true,
            include_remote: true,
            include_token: false,
        };
        let payload =
            build_payload(&app_data, &vault_dir, &token_store, &options, None, None).unwrap();

        assert!(payload.sync.as_ref().unwrap().token.is_none());
    }
}
