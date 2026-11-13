//! MCP Raptor Player: reproductor de escritorio con lista de audio en
//! secuencia, mezcla entre temas, botonera de efectos y servidor MCP.

// En las versiones de entrega no se abre una consola junto a la ventana.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// La lista de herramientas MCP es un json! grande: necesita mas profundidad de macro.
#![recursion_limit = "256"]

// Primero: define la macro tr! para el resto de los modulos.
#[macro_use]
mod idioma;
mod audio;
mod bitacora;
#[cfg(test)]
mod generar_icono;
mod dominio;
mod interfaz;
mod mcp;
mod persistencia;
mod servicio;

use audio::{Mezclador, Motor};
use interfaz::SalidaDePreescucha;
use mcp::{PUERTO_POR_DEFECTO, ServidorMcp, VARIABLE_DE_PUERTO};
use persistencia::{RepositorioDeBotonera, RepositorioDeCanciones, RepositorioDeLista, RepositorioDePreferencias};
use servicio::{GestorDeOverlay, GestorDeReproduccion};

slint::include_modules!();

/// Tiempo desde que arranca el ciclo de la interfaz hasta maximizar la
/// ventana, para que el sistema ya la haya creado y mostrado.
const ESPERA_PARA_MAXIMIZAR: std::time::Duration = std::time::Duration::from_millis(150);

/// Identificador de la aplicacion en los escritorios de Linux: coincide con el
/// nombre del lanzador .desktop y del icono instalado.
#[cfg(target_os = "linux")]
const ID_DE_APLICACION: &str = "mcp-raptor-player";

fn main() {
    bitacora::iniciar(&bitacora::carpeta_de_logs(&persistencia::carpeta_del_usuario()));
    let aviso_original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |informacion| {
        log::error!("Error no controlado en la aplicacion: {informacion}");
        aviso_original(informacion);
    }));

    if let Err(error) = ejecutar() {
        interfaz::mostrar_error(&error);
        std::process::exit(1);
    }
}

fn ejecutar() -> Result<(), String> {
    // El idioma se fija antes que nada: tambien los errores del arranque se
    // muestran en el elegido (o, la primera vez, en el del sistema).
    let preferencias = RepositorioDePreferencias::en_carpeta_del_usuario();
    idioma::establecer(preferencias.idioma().unwrap_or_else(idioma::Idioma::del_sistema));
    log::info!("Idioma de la interfaz: {}", idioma::actual().codigo());
    // Dos motores se alternan para mezclar los temas; el tercero emite los
    // efectos de la botonera por encima y el cuarto es la preescucha del
    // editor de pistas (si no suena por otra salida elegida en el editor).
    let (mezclador, voces) =
        Mezclador::abrir(4).map_err(|error| tr!("No hay una salida de audio disponible. {error}", error = error))?;
    let mut motores = voces.into_iter().map(|voz| Motor::new(voz, mezclador.frecuencia()));
    let mut siguiente_motor = || motores.next().expect("el mezclador abre cuatro voces");
    let (a, b, efectos, preescucha) = (siguiente_motor(), siguiente_motor(), siguiente_motor(), siguiente_motor());
    // La musica (tambien la de la preescucha) baja mientras suena un efecto.
    for musica in [&a, &b, &preescucha] {
        musica.definir_atenuable(true);
    }

    let carpeta_de_ondas = persistencia::carpeta_del_usuario().join("ondas");

    let ventana = VentanaPrincipal::new().map_err(|error| tr!("No se pudo crear la ventana: {error}", error = error))?;
    // En Linux el escritorio asocia la ventana con su lanzador
    // (recursos/linux/mcp-raptor-player.desktop) por este identificador, y de
    // ahi toma el icono y el nombre de la barra de tareas.
    #[cfg(target_os = "linux")]
    if let Err(error) = slint::set_xdg_app_id(ID_DE_APLICACION) {
        log::warn!("No se pudo fijar el identificador de la aplicacion: {error}");
    }
    let _ciclo = interfaz::conectar(
        &ventana,
        GestorDeReproduccion::new(a, b),
        GestorDeOverlay::new(efectos, mezclador.atenuador(), carpeta_de_ondas.clone()),
        RepositorioDeCanciones::en_carpeta_del_usuario(),
        RepositorioDeBotonera::en_carpeta_del_usuario(),
        preferencias,
        SalidaDePreescucha::new(mezclador, preescucha, carpeta_de_ondas),
        RepositorioDeLista::en_carpeta_del_usuario(),
        persistencia::AlmacenesDePublicidad::en_carpeta_del_usuario(),
    )?;
    let _servidor = iniciar_servidor_mcp();

    // Se maximiza recien con la ventana visible, como el doble clic en la
    // barra de titulo: pedirlo antes de mostrarla (en Windows) la deja marcada
    // como maximizada pero con el tamano preferido de la interfaz.
    let debil = ventana.as_weak();
    slint::Timer::single_shot(ESPERA_PARA_MAXIMIZAR, move || {
        if let Some(ventana) = debil.upgrade() {
            ventana.window().set_maximized(true);
        }
    });
    ventana.run().map_err(|error| tr!("Fallo la ventana: {error}", error = error))
}

/// Levanta el servidor MCP en el puerto de la variable RAPTOR_MCP_PUERTO (0 lo
/// desactiva). Si el puerto esta ocupado el reproductor sigue funcionando,
/// solo que sin control remoto.
fn iniciar_servidor_mcp() -> Option<ServidorMcp> {
    let puerto = match std::env::var(VARIABLE_DE_PUERTO) {
        Ok(valor) => match valor.trim().parse::<u16>() {
            Ok(puerto) => puerto,
            Err(_) => {
                log::warn!("{VARIABLE_DE_PUERTO}={valor} no es un puerto valido; se usa {PUERTO_POR_DEFECTO}");
                PUERTO_POR_DEFECTO
            }
        },
        Err(_) => PUERTO_POR_DEFECTO,
    };
    if puerto == 0 {
        return None;
    }
    ServidorMcp::iniciar(puerto)
        .inspect_err(|error| log::warn!("No se pudo iniciar el servidor MCP en el puerto {puerto}: {error}"))
        .ok()
}
