/// Dashboard application state, execution engine, and task scheduling logic.
pub mod app;
/// Terminal event handling (key presses, resize, tick) for the TUI.
pub mod event;
/// PTY session management — allocates pseudo-terminals for Stargate agent panes.
pub mod pty_session;
/// UAT TUI application state machine for interactive finding capture.
pub mod uat_app;
/// UAT TUI rendering — layouts and widgets for the UAT interface.
pub mod uat_ui;
/// Dashboard rendering — task board, agent output panes, and event log widgets.
pub mod ui;
