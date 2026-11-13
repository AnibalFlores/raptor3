//! Prioridad de los hilos propios frente al resto del sistema.
//!
//! El callback de salida ya corre en tiempo real (lo promueve cpal). Los
//! hilos que lo alimentan van por encima de lo normal, para que el buffer de
//! cada voz no se vacie cuando la maquina esta ocupada, y los analisis van
//! por debajo, para no competir con el audio ni con la interfaz.

/// Para que se usa el hilo que llama a `fijar_prioridad_del_hilo_actual`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrioridadDeHilo {
    /// Decodifica audio que va a sonar: por encima de lo normal.
    Audio,
    /// Analisis y calculos que pueden esperar: por debajo de lo normal y,
    /// en Windows, tambien con baja prioridad de disco.
    Fondo,
}

/// Cambia la prioridad del hilo actual. Si el sistema no lo permite (en
/// Linux, subir la prioridad requiere permisos) el hilo sigue como estaba.
pub fn fijar_prioridad_del_hilo_actual(prioridad: PrioridadDeHilo) {
    if let Err(error) = aplicar(prioridad) {
        log::debug!("No se pudo fijar la prioridad {prioridad:?} del hilo: {error}");
    }
}

#[cfg(windows)]
fn aplicar(prioridad: PrioridadDeHilo) -> Result<(), String> {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY_ABOVE_NORMAL,
    };
    let valor = match prioridad {
        PrioridadDeHilo::Audio => THREAD_PRIORITY_ABOVE_NORMAL,
        PrioridadDeHilo::Fondo => THREAD_MODE_BACKGROUND_BEGIN,
    };
    // SAFETY: GetCurrentThread devuelve un seudo identificador siempre valido
    // para el hilo que llama, y SetThreadPriority no retiene punteros.
    let aplicada = unsafe { SetThreadPriority(GetCurrentThread(), valor) };
    if aplicada == 0 { Err(std::io::Error::last_os_error().to_string()) } else { Ok(()) }
}

#[cfg(target_os = "linux")]
fn aplicar(prioridad: PrioridadDeHilo) -> Result<(), String> {
    let amabilidad = match prioridad {
        PrioridadDeHilo::Audio => -5,
        PrioridadDeHilo::Fondo => 10,
    };
    // SAFETY: gettid y setpriority solo leen y cambian atributos del hilo
    // actual; no reciben punteros.
    let resultado = unsafe { libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, amabilidad) };
    if resultado == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn aplicar(_prioridad: PrioridadDeHilo) -> Result<(), String> {
    Ok(())
}
