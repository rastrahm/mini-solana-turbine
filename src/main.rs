//! Binario de entrada. En la fase 1 no abre sockets ni lee el disco.

use mini_solana_turbine::Error;

/// Purpose: Punto de entrada del proceso. En la fase 1 no realiza I/O.
/// Inputs: none (no se leen argumentos ni stdin).
/// Returns: `Ok(())` siempre; el tipo `Result` deja sitio al pipeline posterior.
fn main() -> Result<(), Error> {
    Ok(())
}
