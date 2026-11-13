//! Archivos de audio de una carpeta, con o sin sus subcarpetas.

use std::path::{Path, PathBuf};

use super::Pista;

/// Lo que se encontro al recorrer una carpeta.
#[derive(Debug, Default, PartialEq)]
pub struct AudiosDeCarpeta {
    /// Archivos de audio en orden: los de cada carpeta por nombre y despues
    /// los de sus subcarpetas, tambien por nombre.
    pub archivos: Vec<PathBuf>,
    /// Carpetas recorridas, contando la elegida.
    pub carpetas: usize,
    /// Subcarpetas que no se pudieron leer (sin permiso, desconectadas).
    pub ilegibles: usize,
}

/// Clave para ordenar por nombre sin distinguir mayusculas.
fn clave_de_orden(ruta: &Path) -> String {
    ruta.file_name().map(|nombre| nombre.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Recorre la carpeta (y, con `con_subcarpetas`, todas las que contiene) y
/// junta los archivos de audio. Los enlaces a carpetas no se siguen, para no
/// recorrer dos veces lo mismo ni entrar en un ciclo. Falla solo si no se
/// puede leer la carpeta elegida.
pub fn audios_de_carpeta(carpeta: &Path, con_subcarpetas: bool) -> std::io::Result<AudiosDeCarpeta> {
    let mut resultado = AudiosDeCarpeta::default();
    recorrer(carpeta, con_subcarpetas, &mut resultado, true)?;
    Ok(resultado)
}

fn recorrer(carpeta: &Path, con_subcarpetas: bool, resultado: &mut AudiosDeCarpeta, es_la_elegida: bool) -> std::io::Result<()> {
    let entradas = match std::fs::read_dir(carpeta) {
        Ok(entradas) => entradas,
        Err(error) if es_la_elegida => return Err(error),
        Err(_) => {
            resultado.ilegibles += 1;
            return Ok(());
        }
    };
    resultado.carpetas += 1;
    let mut archivos = Vec::new();
    let mut subcarpetas = Vec::new();
    for entrada in entradas.flatten() {
        let Ok(tipo) = entrada.file_type() else {
            continue;
        };
        let ruta = entrada.path();
        if tipo.is_dir() {
            subcarpetas.push(ruta);
        } else if tipo.is_file() && Pista::es_formato_soportado(&ruta) {
            archivos.push(ruta);
        }
    }
    archivos.sort_by_key(|ruta| clave_de_orden(ruta));
    resultado.archivos.extend(archivos);
    if con_subcarpetas {
        subcarpetas.sort_by_key(|ruta| clave_de_orden(ruta));
        for subcarpeta in subcarpetas {
            recorrer(&subcarpeta, true, resultado, false)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn crear(ruta: &Path) {
        std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
        std::fs::write(ruta, b"audio").unwrap();
    }

    #[test]
    fn recorre_las_subcarpetas_en_orden_y_solo_toma_audio() {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-carpetas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&carpeta);
        crear(&carpeta.join("b.mp3"));
        crear(&carpeta.join("A.flac"));
        crear(&carpeta.join("notas.txt"));
        crear(&carpeta.join("Rock/z.mp3"));
        crear(&carpeta.join("Rock/80s/m.wav"));
        crear(&carpeta.join("pop/y.ogg"));
        std::fs::create_dir_all(carpeta.join("vacia")).unwrap();

        let nombres = |resultado: &AudiosDeCarpeta| -> Vec<String> {
            resultado
                .archivos
                .iter()
                .map(|ruta| ruta.strip_prefix(&carpeta).unwrap().to_string_lossy().replace('\\', "/"))
                .collect()
        };
        let solo_la_carpeta = audios_de_carpeta(&carpeta, false).unwrap();
        assert_eq!(nombres(&solo_la_carpeta), ["A.flac", "b.mp3"]);
        assert_eq!(solo_la_carpeta.carpetas, 1);

        let con_subcarpetas = audios_de_carpeta(&carpeta, true).unwrap();
        assert_eq!(nombres(&con_subcarpetas), ["A.flac", "b.mp3", "pop/y.ogg", "Rock/z.mp3", "Rock/80s/m.wav"]);
        assert_eq!(con_subcarpetas.carpetas, 5);
        assert_eq!(con_subcarpetas.ilegibles, 0);

        assert!(audios_de_carpeta(&carpeta.join("no-existe"), true).is_err());
        let _ = std::fs::remove_dir_all(&carpeta);
    }
}
