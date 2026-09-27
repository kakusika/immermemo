//! Encrypts the access token with a key that never leaves the Android
//! Keystore, reached over JNI: `java.security.KeyStore` (the
//! `"AndroidKeyStore"` provider) and `javax.crypto.Cipher`, both plain
//! platform APIs, so nothing here needs a Gradle-managed dependency.
//!
//! This module can only be exercised by running the app on a device -- there
//! is no way to run real JNI calls in this workspace's own tests, and a
//! typo in a JNI method signature fails at runtime, not at compile time.
//! What *is* tested on desktop is the blob format around the ciphertext
//! ([`crate::token_blob`]).

use std::path::PathBuf;

use anyhow::{Context, Result};
use jni::objects::{JObject, JString, JValue};
use jni::sys::jint;
use jni::{JNIEnv, JavaVM};

use crate::credentials::TokenStore;
use crate::token_blob;

/// The alias the key is filed under in the Keystore. Fixed: this app has
/// exactly one token to protect.
const KEY_ALIAS: &str = "immermemo-remote-token";

/// `android.security.keystore.KeyProperties.PURPOSE_ENCRYPT | PURPOSE_DECRYPT`.
/// Hardcoded rather than read over JNI: these are long-stable public API
/// constants (1 and 2), and skipping the lookup is one less signature that
/// can be gotten wrong.
const PURPOSE_ENCRYPT_AND_DECRYPT: jint = 1 | 2;
/// `javax.crypto.Cipher.ENCRYPT_MODE` / `DECRYPT_MODE`, same reasoning.
const CIPHER_ENCRYPT_MODE: jint = 1;
const CIPHER_DECRYPT_MODE: jint = 2;

const TRANSFORMATION: &str = "AES/GCM/NoPadding";
const GCM_TAG_BITS: jint = 128;

pub struct AndroidKeystoreTokenStore {
    vm: JavaVM,
    path: PathBuf,
}

impl AndroidKeystoreTokenStore {
    /// # Safety
    /// `vm_ptr` must be the `JavaVM*` an `AndroidApp` hands out
    /// (`app.vm_as_ptr()`), valid for the process's lifetime.
    pub unsafe fn new(vm_ptr: *mut std::ffi::c_void, path: PathBuf) -> Result<Self> {
        let vm = unsafe { JavaVM::from_raw(vm_ptr.cast())? };
        Ok(Self { vm, path })
    }

    /// Loads the AES key from the Keystore, generating it on first use.
    fn secret_key<'a>(&self, env: &mut JNIEnv<'a>) -> Result<JObject<'a>> {
        let ks_class = env.find_class("java/security/KeyStore")?;
        let provider = env.new_string("AndroidKeyStore")?;
        let ks = env
            .call_static_method(
                ks_class,
                "getInstance",
                "(Ljava/lang/String;)Ljava/security/KeyStore;",
                &[JValue::Object(&provider)],
            )?
            .l()?;
        env.call_method(
            &ks,
            "load",
            "(Ljava/security/KeyStore$LoadStoreParameter;)V",
            &[JValue::Object(&JObject::null())],
        )?;
        check_exception(env, "KeyStore.load")?;

        let alias = env.new_string(KEY_ALIAS)?;
        let has_key = env
            .call_method(
                &ks,
                "containsAlias",
                "(Ljava/lang/String;)Z",
                &[JValue::Object(&alias)],
            )?
            .z()?;
        if !has_key {
            self.generate_key(env, &alias)?;
        }

        let key = env
            .call_method(
                &ks,
                "getKey",
                "(Ljava/lang/String;[C)Ljava/security/Key;",
                &[JValue::Object(&alias), JValue::Object(&JObject::null())],
            )?
            .l()?;
        check_exception(env, "KeyStore.getKey")?;
        Ok(key)
    }

    fn generate_key(&self, env: &mut JNIEnv, alias: &JString) -> Result<()> {
        let kg_class = env.find_class("javax/crypto/KeyGenerator")?;
        let algo = env.new_string("AES")?;
        let provider = env.new_string("AndroidKeyStore")?;
        let kg = env
            .call_static_method(
                kg_class,
                "getInstance",
                "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;",
                &[JValue::Object(&algo), JValue::Object(&provider)],
            )?
            .l()?;
        check_exception(env, "KeyGenerator.getInstance")?;

        let builder_class =
            env.find_class("android/security/keystore/KeyGenParameterSpec$Builder")?;
        let builder = env.new_object(
            builder_class,
            "(Ljava/lang/String;I)V",
            &[
                JValue::Object(alias),
                JValue::Int(PURPOSE_ENCRYPT_AND_DECRYPT),
            ],
        )?;
        check_exception(env, "KeyGenParameterSpec.Builder")?;

        let gcm = env.new_string("GCM")?;
        let block_modes = env.new_object_array(1, "java/lang/String", &gcm)?;
        env.call_method(
            &builder,
            "setBlockModes",
            "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
            &[JValue::Object(&block_modes)],
        )?;
        check_exception(env, "KeyGenParameterSpec.Builder.setBlockModes")?;

        let no_padding = env.new_string("NoPadding")?;
        let paddings = env.new_object_array(1, "java/lang/String", &no_padding)?;
        env.call_method(
            &builder,
            "setEncryptionPaddings",
            "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
            &[JValue::Object(&paddings)],
        )?;
        check_exception(env, "KeyGenParameterSpec.Builder.setEncryptionPaddings")?;

        let spec = env
            .call_method(
                &builder,
                "build",
                "()Landroid/security/keystore/KeyGenParameterSpec;",
                &[],
            )?
            .l()?;
        check_exception(env, "KeyGenParameterSpec.Builder.build")?;

        env.call_method(
            &kg,
            "init",
            "(Ljava/security/spec/AlgorithmParameterSpec;)V",
            &[JValue::Object(&spec)],
        )?;
        check_exception(env, "KeyGenerator.init")?;

        env.call_method(&kg, "generateKey", "()Ljavax/crypto/SecretKey;", &[])?;
        check_exception(env, "KeyGenerator.generateKey")
    }

    fn cipher<'a>(&self, env: &mut JNIEnv<'a>) -> Result<JObject<'a>> {
        let cipher_class = env.find_class("javax/crypto/Cipher")?;
        let transformation = env.new_string(TRANSFORMATION)?;
        let cipher = env
            .call_static_method(
                cipher_class,
                "getInstance",
                "(Ljava/lang/String;)Ljavax/crypto/Cipher;",
                &[JValue::Object(&transformation)],
            )?
            .l()?;
        check_exception(env, "Cipher.getInstance")?;
        Ok(cipher)
    }

    fn encrypt(&self, env: &mut JNIEnv, plaintext: &[u8]) -> Result<Vec<u8>> {
        let key = self.secret_key(env)?;
        let cipher = self.cipher(env)?;
        env.call_method(
            &cipher,
            "init",
            "(ILjava/security/Key;)V",
            &[JValue::Int(CIPHER_ENCRYPT_MODE), JValue::Object(&key)],
        )?;
        check_exception(env, "Cipher.init(ENCRYPT_MODE)")?;

        let iv_obj = env.call_method(&cipher, "getIV", "()[B", &[])?.l()?;
        let iv = env.convert_byte_array(jni::objects::JByteArray::from(iv_obj))?;

        let input = env.byte_array_from_slice(plaintext)?;
        let output = env
            .call_method(&cipher, "doFinal", "([B)[B", &[JValue::Object(&input)])?
            .l()?;
        check_exception(env, "Cipher.doFinal(encrypt)")?;
        let ciphertext = env.convert_byte_array(jni::objects::JByteArray::from(output))?;

        Ok(token_blob::encode(&iv, &ciphertext))
    }

    fn decrypt(&self, env: &mut JNIEnv, blob: &[u8]) -> Result<Vec<u8>> {
        let (iv, ciphertext) = token_blob::decode(blob)?;

        let key = self.secret_key(env)?;
        let cipher = self.cipher(env)?;

        let spec_class = env.find_class("javax/crypto/spec/GCMParameterSpec")?;
        let iv_array = env.byte_array_from_slice(iv)?;
        let spec = env.new_object(
            spec_class,
            "(I[B)V",
            &[JValue::Int(GCM_TAG_BITS), JValue::Object(&iv_array)],
        )?;
        check_exception(env, "GCMParameterSpec")?;

        env.call_method(
            &cipher,
            "init",
            "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V",
            &[
                JValue::Int(CIPHER_DECRYPT_MODE),
                JValue::Object(&key),
                JValue::Object(&spec),
            ],
        )?;
        check_exception(env, "Cipher.init(DECRYPT_MODE)")?;

        let input = env.byte_array_from_slice(ciphertext)?;
        let output = env
            .call_method(&cipher, "doFinal", "([B)[B", &[JValue::Object(&input)])?
            .l()?;
        check_exception(env, "Cipher.doFinal(decrypt)")?;
        Ok(env.convert_byte_array(jni::objects::JByteArray::from(output))?)
    }
}

impl TokenStore for AndroidKeystoreTokenStore {
    fn load(&self) -> Result<Option<String>> {
        let blob = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut env = self.vm.attach_current_thread()?;
        let plaintext = self
            .decrypt(&mut env, &blob)
            .context("decrypting the saved token")?;
        Ok(Some(String::from_utf8(plaintext)?))
    }

    fn save(&self, token: &str) -> Result<()> {
        let mut env = self.vm.attach_current_thread()?;
        let blob = self
            .encrypt(&mut env, token.as_bytes())
            .context("encrypting the token")?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, blob)?;
        Ok(())
    }
}

/// If a Java exception is pending, turns it into an `anyhow::Error` carrying
/// its message (also left on logcat via `exception_describe`) instead of
/// letting it cross back into Rust, where the `jni` crate would otherwise
/// abort the process.
fn check_exception(env: &mut JNIEnv, what: &str) -> Result<()> {
    if !env.exception_check()? {
        return Ok(());
    }
    let thrown = env.exception_occurred()?;
    env.exception_describe()?;
    env.exception_clear()?;
    let message = env
        .call_method(&thrown, "toString", "()Ljava/lang/String;", &[])
        .ok()
        .and_then(|v| v.l().ok())
        .map(JString::from)
        .and_then(|s| env.get_string(&s).ok().map(String::from))
        .unwrap_or_else(|| "<no message>".to_owned());
    anyhow::bail!("{what}: {message}")
}
