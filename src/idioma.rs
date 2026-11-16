//! Idioma de la interfaz. Los textos se escriben en español (el idioma de
//! origen) y sus traducciones viven en `lang/<codigo>/LC_MESSAGES/
//! mcp-raptor-player.po`, el mismo catalogo gettext para las dos partes:
//!
//! - La interfaz Slint marca sus textos con `@tr("...")` y Slint los cambia al
//!   elegir el idioma con `slint::select_bundled_translation`.
//! - El codigo Rust usa la macro `tr!` (como `format!`, con los argumentos
//!   explicitos): `tr!("No se pudo leer {ruta}: {error}", ruta = ..., error = error)`.
//!   El catalogo se convierte al compilar en la macro `traduccion_en!`, asi
//!   una traduccion con un marcador que no existe no compila.
//!
//! `herramientas/traducciones.py` junta los textos marcados y actualiza el
//! catalogo; un test verifica que no falte ninguna traduccion.

use std::sync::atomic::{AtomicU8, Ordering};

// Define `traduccion_en!`: de cada texto en español a su traduccion al
// inglés (o el mismo texto si todavia no tiene).
include!(concat!(env!("OUT_DIR"), "/traducciones.rs"));

/// Texto traducido al idioma de la interfaz. Sin argumentos devuelve el texto;
/// con argumentos se comporta como `format!` (los marcadores deben pasarse
/// explicitamente, por nombre o por posicion).
///
/// La plantilla se recibe como `tt` y no como `literal`: un fragmento
/// `literal` llega opaco a `traduccion_en!`, que entonces no lo reconoce y
/// lo dejaria siempre en español.
macro_rules! tr {
    ($plantilla:tt) => {
        if $crate::idioma::en_ingles() { tr_en!($plantilla) } else { String::from($plantilla) }
    };
    ($plantilla:tt, $($argumentos:tt)+) => {
        if $crate::idioma::en_ingles() { tr_en!($plantilla, $($argumentos)+) } else { format!($plantilla, $($argumentos)+) }
    };
}

/// El texto en inglés, sin mirar el idioma elegido (lo usa `tr!`).
macro_rules! tr_en {
    ($plantilla:tt) => {
        String::from(traduccion_en!($plantilla))
    };
    ($plantilla:tt, $($argumentos:tt)+) => {
        format!(traduccion_en!($plantilla), $($argumentos)+)
    };
}

/// Idiomas de la interfaz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Idioma {
    #[default]
    Espanol,
    Ingles,
}

impl Idioma {
    /// Todos, en el orden en que se ofrecen.
    pub const TODOS: [Self; 2] = [Self::Espanol, Self::Ingles];

    /// Codigo ISO 639-1, el que se guarda en las preferencias.
    pub fn codigo(self) -> &'static str {
        match self {
            Self::Espanol => "es",
            Self::Ingles => "en",
        }
    }

    /// Nombre del idioma en ese mismo idioma (asi se reconoce en el menu
    /// aunque la interfaz este en otro).
    pub fn nombre(self) -> &'static str {
        match self {
            Self::Espanol => "Español",
            Self::Ingles => "English",
        }
    }

    /// El idioma de un codigo como "es", "en-US" o "en_GB.UTF-8"; None si no
    /// es uno de los disponibles.
    pub fn desde_codigo(codigo: &str) -> Option<Self> {
        let principal = codigo.trim().split(['-', '_', '.', '@']).next().unwrap_or_default().to_ascii_lowercase();
        Self::TODOS.into_iter().find(|idioma| idioma.codigo() == principal)
    }

    /// Idioma de la cuenta del sistema: inglés si lo es; si no (o si no se
    /// puede saber), español.
    pub fn del_sistema() -> Self {
        codigo_del_sistema().as_deref().and_then(Self::desde_codigo).unwrap_or_default()
    }

    /// Codigo de idioma que Slint usa para elegir la traduccion: la interfaz
    /// esta escrita en español, asi que el texto original se elige con "".
    pub fn codigo_para_slint(self) -> &'static str {
        match self {
            Self::Espanol => "",
            Self::Ingles => "en",
        }
    }
}

static ACTUAL: AtomicU8 = AtomicU8::new(0);

/// Idioma en que se muestran los textos.
pub fn actual() -> Idioma {
    if ACTUAL.load(Ordering::Relaxed) == 1 { Idioma::Ingles } else { Idioma::Espanol }
}

/// Cambia el idioma de los textos que arma el codigo Rust (los de Slint se
/// cambian aparte, con `slint::select_bundled_translation`).
pub fn establecer(idioma: Idioma) {
    ACTUAL.store(u8::from(idioma == Idioma::Ingles), Ordering::Relaxed);
}

pub fn en_ingles() -> bool {
    actual() == Idioma::Ingles
}

#[cfg(windows)]
fn codigo_del_sistema() -> Option<String> {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    // LOCALE_NAME_MAX_LENGTH es 85.
    let mut nombre = [0u16; 85];
    // SAFETY: el bufer es valido y se pasa su largo exacto.
    let largo = unsafe { GetUserDefaultLocaleName(nombre.as_mut_ptr(), nombre.len() as i32) };
    (largo > 1).then(|| String::from_utf16_lossy(&nombre[..largo as usize - 1]))
}

/// Idioma de las variables de entorno (Linux, o macOS desde la Terminal).
#[cfg(not(windows))]
fn codigo_del_entorno() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().filter_map(|variable| std::env::var(variable).ok()).find(|valor| !valor.is_empty())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn codigo_del_sistema() -> Option<String> {
    codigo_del_entorno()
}

/// En macOS una app abierta desde el Finder no recibe LANG: el idioma se lee
/// de la preferencia del sistema (AppleLanguages), el primero de la lista.
#[cfg(target_os = "macos")]
fn codigo_del_sistema() -> Option<String> {
    codigo_del_entorno().or_else(|| {
        let salida = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok()?;
        primer_idioma_de_apple(&String::from_utf8_lossy(&salida.stdout))
    })
}

/// El primer idioma de la lista que imprime `defaults read -g AppleLanguages`:
/// `( "en-AR", "es-AR" )`, con o sin comillas y en varias lineas.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn primer_idioma_de_apple(texto: &str) -> Option<String> {
    texto
        .split(['(', ')', ',', '\n'])
        .map(|parte| parte.trim().trim_matches('"').trim())
        .find(|parte| !parte.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn reconoce_los_codigos_de_idioma() {
        assert_eq!(Idioma::desde_codigo("es"), Some(Idioma::Espanol));
        assert_eq!(Idioma::desde_codigo("es-AR"), Some(Idioma::Espanol));
        assert_eq!(Idioma::desde_codigo("en_US.UTF-8"), Some(Idioma::Ingles));
        assert_eq!(Idioma::desde_codigo("EN"), Some(Idioma::Ingles));
        assert_eq!(Idioma::desde_codigo("fr-FR"), None);
        assert_eq!(Idioma::desde_codigo(""), None);
    }

    #[test]
    fn lee_el_primer_idioma_de_la_preferencia_de_macos() {
        let salida = "(\n    \"en-AR\",\n    \"es-AR\"\n)\n";
        assert_eq!(primer_idioma_de_apple(salida).as_deref(), Some("en-AR"));
        assert_eq!(primer_idioma_de_apple("(\n    es,\n    en\n)").as_deref(), Some("es"));
        assert_eq!(primer_idioma_de_apple("").as_deref(), None);
        assert_eq!(Idioma::desde_codigo(&primer_idioma_de_apple(salida).unwrap()), Some(Idioma::Ingles));
    }

    /// La macro generada al compilar lleva cada texto a su traduccion (sin
    /// tocar el idioma global, que comparten las demas pruebas).
    #[test]
    fn la_macro_traduce_y_conserva_los_marcadores() {
        assert_eq!(traduccion_en!("Cancelar"), "Cancel");
        assert_eq!(format!(traduccion_en!("Escribir a {}"), "Raptor"), "Write to Raptor");
        let nombre = "Tanda A";
        assert_eq!(format!(traduccion_en!("No hay una tanda llamada «{nombre}»."), nombre = nombre), "There is no break called «Tanda A».");
        // Sin traduccion queda el texto original.
        assert_eq!(traduccion_en!("texto que no esta en el catalogo"), "texto que no esta en el catalogo");
        // Por el mismo camino que usa tr! (la plantilla pasa de macro en macro).
        assert_eq!(tr_en!("Sin pista cargada"), "No track loaded");
        assert_eq!(tr_en!("Tanda «{}» quitada.", "A"), "Tanda «A» quitada.");
        assert_eq!(tr_en!("No se pudo abrir {nombre}: {error}", nombre = "x.mp3", error = "e"), "Could not open x.mp3: e");
    }

    /// Cada texto marcado para traducir tiene su traduccion al inglés, con
    /// los mismos marcadores.
    #[test]
    fn el_catalogo_en_ingles_esta_completo() {
        let catalogo = include_str!("../lang/en/LC_MESSAGES/mcp-raptor-player.po");
        let entradas = leer_po(catalogo);
        assert!(!entradas.is_empty());
        let marcadores = |texto: &str| {
            let mut encontrados: Vec<String> = Vec::new();
            let mut resto = texto.replace("{{", "").replace("}}", "");
            while let Some(inicio) = resto.find('{') {
                let Some(fin) = resto[inicio..].find('}') else { break };
                encontrados.push(resto[inicio..inicio + fin + 1].to_string());
                resto = resto[inicio + fin + 1..].to_string();
            }
            encontrados.sort();
            encontrados
        };
        let faltan: Vec<&str> = entradas.iter().filter(|(_, ingles)| ingles.is_empty()).map(|(espanol, _)| espanol.as_str()).collect();
        assert!(faltan.is_empty(), "Faltan {} traducciones: {:?}", faltan.len(), &faltan[..faltan.len().min(10)]);
        for (espanol, ingles) in &entradas {
            assert_eq!(marcadores(espanol), marcadores(ingles), "Marcadores distintos en «{espanol}»");
        }
    }

    /// Pares (msgid, msgstr) de un catalogo .po (sin contexto ni plurales).
    fn leer_po(texto: &str) -> Vec<(String, String)> {
        let mut entradas = Vec::new();
        let (mut id, mut traduccion, mut campo) = (String::new(), String::new(), 0);
        let cerrar = |id: &mut String, traduccion: &mut String, entradas: &mut Vec<(String, String)>| {
            if !id.is_empty() {
                entradas.push((std::mem::take(id), std::mem::take(traduccion)));
            }
            id.clear();
            traduccion.clear();
        };
        for linea in texto.lines().map(str::trim) {
            let cadena = |resto: &str| resto.trim().trim_matches('"').to_string();
            if let Some(resto) = linea.strip_prefix("msgid ") {
                cerrar(&mut id, &mut traduccion, &mut entradas);
                id = cadena(resto);
                campo = 1;
            } else if let Some(resto) = linea.strip_prefix("msgstr ") {
                traduccion = cadena(resto);
                campo = 2;
            } else if linea.starts_with('"') {
                match campo {
                    1 => id.push_str(&cadena(linea)),
                    2 => traduccion.push_str(&cadena(linea)),
                    _ => {}
                }
            }
        }
        cerrar(&mut id, &mut traduccion, &mut entradas);
        entradas
    }
}
