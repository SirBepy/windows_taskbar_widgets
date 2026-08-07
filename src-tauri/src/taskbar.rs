mod monitors;
mod rect;

// Glob, not named re-exports: #[tauri::command] also emits a hidden sibling item
// per function that generate_handler! needs resolvable at this same path.
pub use monitors::*;
pub use rect::*;
