//! Mattoni condivisi tra i tool esterni integrati in AetherDesk
//! (Steamless, UCOnline2, ...).
//!
//! Ogni tool mantiene la propria logica specifica (runner per Steamless,
//! deployer per UCOnline2) nel proprio modulo; qui vive solo ciò che è
//! davvero identico tra loro: locazione/installazione del bundle, utilità
//! filesystem e costanti di contratto. Regola: se un pezzo serve a due
//! tool, sta qui; se serve a uno solo, sta nel modulo del tool.

pub mod bundle;
pub mod constants;
pub mod fs;
