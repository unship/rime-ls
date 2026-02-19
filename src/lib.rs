/// handle config and settings
pub mod config;
/// document-based temporary dictionary (segment file, use as extra candidates)
pub mod document_dict;
/// file-based logging
pub mod logger;
/// watch Rime user_data_dir for config changes
mod watcher;
/// const values
mod consts;
/// handle user input
mod input;
/// librime C FFI
pub mod rime;
/// helper functions
pub mod utils;

/// impl LSP for Rime
pub mod lsp;
