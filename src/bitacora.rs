//! Bitacora de diagnostico en ~/.mcp-raptor-player/logs/raptor.log.
//!
//! La version de entrega no abre consola, asi que los mensajes del registro
//! (`log::info!`, `log::warn!`, ...) se escriben tambien en un archivo. Al
//! pasar de `TAMANO_MAXIMO` bytes el archivo rota: raptor.log pasa a
//! raptor.1.log, raptor.1.log a raptor.2.log, y asi hasta `ARCHIVOS_VIEJOS`;
//! el mas viejo se descarta.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Tamano a partir del cual el archivo rota.
const TAMANO_MAXIMO: u64 = 5 * 1024 * 1024;
/// Archivos rotados que se conservan, ademas del actual.
const ARCHIVOS_VIEJOS: usize = 3;
const NOMBRE: &str = "raptor";

/// Carpeta de la bitacora dentro de la carpeta de datos.
pub fn carpeta_de_logs(carpeta_de_datos: &Path) -> PathBuf {
    carpeta_de_datos.join("logs")
}

/// Inicia el registro: nivel `info` por defecto (o el de RUST_LOG), hacia el
/// archivo rotativo y, en las versiones de desarrollo, tambien a la consola.
/// Si no se puede crear el archivo, solo se usa la consola.
pub fn iniciar(carpeta: &Path) {
    let mut constructor = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    constructor.format(|salida, registro| {
        writeln!(
            salida,
            "{} {:<5} [{}] {}",
            jiff::Zoned::now().strftime("%Y-%m-%d %H:%M:%S%.3f"),
            registro.level(),
            registro.target(),
            registro.args()
        )
    });
    match ArchivoRotativo::abrir(carpeta) {
        Ok(archivo) => {
            let destino = Duplicado { archivo, consola: cfg!(debug_assertions) };
            constructor.target(env_logger::Target::Pipe(Box::new(destino)));
        }
        Err(error) => eprintln!("No se pudo abrir la bitacora en {}: {error}", carpeta.display()),
    }
    constructor.init();
}

/// Escribe en el archivo y, si se pide, tambien en la consola.
struct Duplicado {
    archivo: ArchivoRotativo,
    consola: bool,
}

impl Write for Duplicado {
    fn write(&mut self, datos: &[u8]) -> io::Result<usize> {
        if self.consola {
            let _ = io::stderr().write_all(datos);
        }
        self.archivo.write_all(datos)?;
        Ok(datos.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.archivo.flush()
    }
}

/// Archivo que rota por tamano.
struct ArchivoRotativo {
    carpeta: PathBuf,
    archivo: File,
    escritos: u64,
}

impl ArchivoRotativo {
    fn abrir(carpeta: &Path) -> io::Result<Self> {
        fs::create_dir_all(carpeta)?;
        let ruta = ruta_numerada(carpeta, 0);
        let archivo = OpenOptions::new().create(true).append(true).open(&ruta)?;
        let escritos = archivo.metadata().map(|datos| datos.len()).unwrap_or(0);
        Ok(Self { carpeta: carpeta.to_path_buf(), archivo, escritos })
    }

    /// Corre cada archivo un numero y empieza uno vacio.
    fn rotar(&mut self) -> io::Result<()> {
        self.archivo.flush()?;
        let _ = fs::remove_file(ruta_numerada(&self.carpeta, ARCHIVOS_VIEJOS));
        for numero in (0..ARCHIVOS_VIEJOS).rev() {
            let origen = ruta_numerada(&self.carpeta, numero);
            if origen.exists() {
                fs::rename(&origen, ruta_numerada(&self.carpeta, numero + 1))?;
            }
        }
        self.archivo = OpenOptions::new().create(true).write(true).truncate(true).open(ruta_numerada(&self.carpeta, 0))?;
        self.escritos = 0;
        Ok(())
    }
}

impl Write for ArchivoRotativo {
    fn write(&mut self, datos: &[u8]) -> io::Result<usize> {
        if self.escritos > 0 && self.escritos + datos.len() as u64 > TAMANO_MAXIMO {
            // Si no se puede rotar (otro proceso lo tiene abierto en Windows)
            // se sigue escribiendo en el mismo archivo.
            if let Err(error) = self.rotar() {
                eprintln!("No se pudo rotar la bitacora: {error}");
            }
        }
        let escritos = self.archivo.write(datos)?;
        self.escritos += escritos as u64;
        Ok(escritos)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.archivo.flush()
    }
}

/// raptor.log para el actual; raptor.N.log para los rotados.
fn ruta_numerada(carpeta: &Path, numero: usize) -> PathBuf {
    match numero {
        0 => carpeta.join(format!("{NOMBRE}.log")),
        numero => carpeta.join(format!("{NOMBRE}.{numero}.log")),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_archivo_rota_al_superar_el_tamano_y_conserva_los_ultimos() {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-bitacora-{}", std::process::id()));
        let _ = fs::remove_dir_all(&carpeta);
        let mut archivo = ArchivoRotativo::abrir(&carpeta).unwrap();
        let linea = vec![b'x'; (TAMANO_MAXIMO / 2) as usize];
        // Cada dos lineas se llena un archivo: 10 lineas hacen varias rotaciones.
        for _ in 0..10 {
            archivo.write_all(&linea).unwrap();
        }
        archivo.flush().unwrap();
        for numero in 0..=ARCHIVOS_VIEJOS {
            let tamano = fs::metadata(ruta_numerada(&carpeta, numero)).unwrap().len();
            assert!(tamano <= TAMANO_MAXIMO, "raptor.{numero}: {tamano}");
        }
        assert!(!ruta_numerada(&carpeta, ARCHIVOS_VIEJOS + 1).exists(), "el mas viejo se descarta");
        drop(archivo);
        let _ = fs::remove_dir_all(&carpeta);
    }
}
