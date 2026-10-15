# Task: Reorganize Settings Screen

## Context & Motivation
Reorganize the settings screen in `immermemo` to move "Accounts" and "Vaults" out of the "Other" section and into dedicated, logical categories. The new structure organizes settings into four clean sections:
1. **Vaults & Sync** (Vaults, Accounts, Transfer & Share)
2. **Appearance** (Theme, Editor Font Size, UI Scale, Status Bar)
3. **Navigation & Views** (Bottom Navigation Bar, Directory, Daily Notes, Starred, Recent, Graph View)
4. **Other & Info** (Advanced, About)

## Steps
- [x] 1. Create dedicated Jujutsu workspace `.agents/workspaces/settings-reorg`
- [x] 2. Update `app/ui/sheets/settings/settings.slint`:
  - [x] 2.1 Reorganize Top Level (page 0) into the 4 new sections
  - [x] 2.2 Refactor page 1 into Appearance-only (Theme, Font Size, UI Scale, Status Bar) or embed them cleanly
  - [x] 2.3 Create or route Navigation & Views submenu (page 7) with Bottom Nav and Page Settings
  - [x] 2.4 Update top bar title and `go-back()` logic for back navigation
- [x] 3. Verify compilation and tests with `cargo test --workspace` and `cargo build`
- [ ] 4. Clean up Jujutsu workspace and fold into default branch
- [ ] 5. Report completion to user and ask before any commit
