//! Control remoto de la lista por MCP sobre HTTP local.

mod herramientas;
mod servidor;

pub use servidor::{PUERTO_POR_DEFECTO, ServidorMcp, VARIABLE_DE_PUERTO};
