//! Fetch and push over whatever transport the remote URL implies (local
//! path, SSH, HTTPS) -- `git2`'s own job once credentials and the
//! certificate check are wired up.

use git2::{FetchOptions, PushOptions, RemoteCallbacks};
use std::cell::RefCell;
use std::rc::Rc;

use crate::{BRANCH, CredentialProvider, REMOTE_NAME, Vault};

impl Vault {
    pub(crate) fn fetch(&self, credentials: &dyn CredentialProvider) -> anyhow::Result<()> {
        let mut remote = self.repo.find_remote(REMOTE_NAME)?;
        let url = remote.url().unwrap_or_default().to_string();
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(move |_url, _username, _allowed| {
            credentials
                .credentials(&url)
                .map_err(|e| git2::Error::from_str(&e.to_string()))
        });
        self.install_certificate_check(&mut callbacks);
        let mut opts = FetchOptions::new();
        opts.remote_callbacks(callbacks);
        remote.fetch(
            &[format!(
                "+refs/heads/{BRANCH}:refs/remotes/{REMOTE_NAME}/{BRANCH}"
            )],
            Some(&mut opts),
            None,
        )?;
        Ok(())
    }

    /// Pushes the local branch. Returns `Ok(true)` if it was accepted,
    /// `Ok(false)` if the remote rejected it (moved since our last fetch)
    /// so the caller can fetch, reconcile and retry.
    pub(crate) fn push(&self, credentials: &dyn CredentialProvider) -> anyhow::Result<bool> {
        let mut remote = self.repo.find_remote(REMOTE_NAME)?;
        let url = remote.url().unwrap_or_default().to_string();
        let rejected: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        let rejected_in_callback = Rc::clone(&rejected);

        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(move |_url, _username, _allowed| {
            credentials
                .credentials(&url)
                .map_err(|e| git2::Error::from_str(&e.to_string()))
        });
        callbacks.push_update_reference(move |_refname, status| {
            if status.is_some() {
                *rejected_in_callback.borrow_mut() = true;
            }
            Ok(())
        });
        self.install_certificate_check(&mut callbacks);

        let mut opts = PushOptions::new();
        opts.remote_callbacks(callbacks);
        remote.push(
            &[format!("refs/heads/{BRANCH}:refs/heads/{BRANCH}")],
            Some(&mut opts),
        )?;
        Ok(!*rejected.borrow())
    }
}
