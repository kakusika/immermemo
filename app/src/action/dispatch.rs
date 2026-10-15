//! Action dispatching logic.

use std::cell::RefCell;
use std::rc::Rc;

use super::id::ActionId;
use crate::App;
use crate::session::Session;

/// Dispatches an action by its ID.
///
/// This invokes the unified Slint action trigger handler, ensuring that
/// UI transitions (page navigation, note-sheet opening, editor focusing)
/// and Rust business logic (syncing, vault switching) execute together.
pub fn dispatch_action(app: &App, _session: &Rc<RefCell<Session>>, action: ActionId) {
    app.invoke_trigger_action(action.to_i32());
}
