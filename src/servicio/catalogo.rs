//! Busqueda de archivos movidos para el mantenimiento del catalogo.
//!
//! El catalogo identifica cada archivo por su ruta. Si el usuario mueve una
//! carpeta, las fichas quedan apuntando a rutas que ya no existen. Esta
//! busqueda recorre una carpeta (con sus subcarpetas) y, para cada ficha sin
//! archivo, busca un archivo de audio con el mismo nombre. Si hay varios con
//! ese nombre, desempata por la duracion guardada en la ficha.
//!
//! No reconoce archivos renombrados: para eso haria falta comparar el
//! contenido de todos los archivos de la carpeta.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::dominio::Pista;

/// Diferencia maxima de duracion (segundos) para considerar que dos archivos
/// con el mismo nombre son el mismo tema.
const TOLERANCIA_DE_DURACION: f64 = 0.5;

/// Archivo que falta: su ruta anterior y la duracion que tenia, si se conoce.
#[derive(Clone, Debug, PartialEq)]
pub struct ArchivoPerdido {
    pub ruta: PathBuf,
    pub duracion: Option<f64>,
}

/// Resultado de buscar un archivo perdido.
#[derive(Clone, Debug, PartialEq)]
pub enum Hallazgo {
    /// Un solo archivo coincide: es este.
    Encontrado(PathBuf),
    /// Hay varios con el mismo nombre y la duracion no alcanza para elegir.
    Varios(usize),
    NoEncontrado,
}

/// Nombre de archivo para comparar: sin distinguir mayusculas.
fn clave_de_nombre(ruta: &Path) -> Option<String> {
    ruta.file_name().map(|nombre| nombre.to_string_lossy().to_lowercase())
}

/// Archivos de audio de la carpeta y sus subcarpetas, por nombre. Las
/// carpetas que no se pueden leer se saltean; los enlaces no se siguen, para
/// no recorrer dos veces lo mismo ni entrar en un ciclo.
fn indice_de_audios(carpeta: &Path) -> HashMap<String, Vec<PathBuf>> {
    let mut indice: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut pendientes = vec![carpeta.to_path_buf()];
    while let Some(actual) = pendientes.pop() {
        let Ok(entradas) = std::fs::read_dir(&actual) else {
            continue;
        };
        for entrada in entradas.flatten() {
            let Ok(tipo) = entrada.file_type() else {
                continue;
            };
            let ruta = entrada.path();
            if tipo.is_dir() {
                pendientes.push(ruta);
            } else if tipo.is_file()
                && Pista::es_formato_soportado(&ruta)
                && let Some(clave) = clave_de_nombre(&ruta)
            {
                indice.entry(clave).or_default().push(ruta);
            }
        }
    }
    indice
}

/// Busca en `carpeta` cada archivo perdido. `duracion_de` mide un candidato;
/// solo se usa cuando hay varios archivos con el mismo nombre.
pub fn buscar_movidos(
    perdidos: &[ArchivoPerdido],
    carpeta: &Path,
    duracion_de: impl Fn(&Path) -> Option<f64>,
) -> Vec<Hallazgo> {
    let indice = indice_de_audios(carpeta);
    perdidos
        .iter()
        .map(|perdido| {
            let candidatos = clave_de_nombre(&perdido.ruta).and_then(|clave| indice.get(&clave)).map_or(&[][..], Vec::as_slice);
            match candidatos {
                [] => Hallazgo::NoEncontrado,
                [unico] => Hallazgo::Encontrado(unico.clone()),
                varios => {
                    let Some(duracion) = perdido.duracion else {
                        return Hallazgo::Varios(varios.len());
                    };
                    let coinciden: Vec<&PathBuf> = varios
                        .iter()
                        .filter(|candidato| {
                            duracion_de(candidato).is_some_and(|otra| (otra - duracion).abs() <= TOLERANCIA_DE_DURACION)
                        })
                        .collect();
                    match coinciden.as_slice() {
                        [unico] => Hallazgo::Encontrado((*unico).clone()),
                        _ => Hallazgo::Varios(varios.len()),
                    }
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn carpeta_temporal(nombre: &str) -> PathBuf {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-catalogo-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&carpeta);
        carpeta
    }

    fn crear(ruta: &Path) {
        std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
        std::fs::write(ruta, b"audio").unwrap();
    }

    #[test]
    fn encuentra_por_nombre_en_subcarpetas_y_desempata_por_duracion() {
        let carpeta = carpeta_temporal("buscar");
        crear(&carpeta.join("rock/Tema Uno.MP3"));
        crear(&carpeta.join("pop/2024/otro.flac"));
        crear(&carpeta.join("a/repetido.mp3"));
        crear(&carpeta.join("b/repetido.mp3"));
        crear(&carpeta.join("c/doble.mp3"));
        crear(&carpeta.join("d/doble.mp3"));
        crear(&carpeta.join("notas/tema dos.txt"));
        let perdido = |ruta: &str, duracion: Option<f64>| ArchivoPerdido { ruta: PathBuf::from(ruta), duracion };
        let perdidos = [
            perdido("D:/viejo/tema uno.mp3", None),
            perdido("D:/viejo/otro.flac", Some(200.0)),
            perdido("D:/viejo/repetido.mp3", Some(180.0)),
            perdido("D:/viejo/doble.mp3", None),
            perdido("D:/viejo/tema dos.mp3", None),
        ];
        // Solo b/repetido.mp3 dura lo mismo que la ficha.
        let en_b = |ruta: &Path| ruta.parent().and_then(Path::file_name).is_some_and(|carpeta| carpeta == "b");
        let duracion_de = |ruta: &Path| Some(if en_b(ruta) { 180.2 } else { 95.0 });
        let hallazgos = buscar_movidos(&perdidos, &carpeta, duracion_de);
        assert_eq!(hallazgos[0], Hallazgo::Encontrado(carpeta.join("rock/Tema Uno.MP3")), "sin distinguir mayusculas");
        assert_eq!(hallazgos[1], Hallazgo::Encontrado(carpeta.join("pop/2024/otro.flac")));
        assert_eq!(hallazgos[2], Hallazgo::Encontrado(carpeta.join("b/repetido.mp3")));
        assert_eq!(hallazgos[3], Hallazgo::Varios(2), "sin duracion no se puede elegir");
        assert_eq!(hallazgos[4], Hallazgo::NoEncontrado, "un .txt no es audio");
        let _ = std::fs::remove_dir_all(&carpeta);
    }
}
