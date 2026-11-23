//! Carátulas de los temas, para el panel de detalle.
//!
//! Se buscan en este orden:
//!
//! 1. La imagen que trae el archivo en sus etiquetas (la tapa, si esta
//!    marcada como tal).
//! 2. Una imagen en la carpeta del tema (cover, folder, front, portada...),
//!    o en la del album si el tema esta en una carpeta de disco ("CD1").
//! 3. Opcional, solo si se activa: en internet, por artista y album (o
//!    titulo), en Deezer, iTunes y MusicBrainz con el Cover Art Archive, en
//!    ese orden. Ninguno pide cuenta ni clave. Lo encontrado (y lo que no se
//!    encontro) se guarda en la carpeta de datos para no volver a preguntar.
//!
//! Una carátula equivocada es peor que ninguna: en internet solo se acepta
//! un resultado del mismo artista y con el mismo nombre de album o tema, sin
//! otra edicion entre parentesis ("Deluxe", "En vivo"). La idea de las tres
//! fuentes y de estas reglas viene del reproductor sonora (GPL-3.0-or-later).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::audio::Decodificador;

/// Lado maximo de la miniatura que se muestra (el doble del cuadro del
/// panel, para pantallas con escala).
pub const LADO_DE_LA_MINIATURA: u32 = 160;
/// Lado maximo de la carátula ampliada que se ve al pasar el puntero.
pub const LADO_DE_LA_AMPLIADA: u32 = 400;
pub(super) const TIEMPO_MAXIMO_DE_LA_CONSULTA: Duration = Duration::from_secs(10);
/// Una imagen de internet mas grande que esto se descarta.
const TAMANO_MAXIMO_DE_LA_IMAGEN: u64 = 8 * 1024 * 1024;
/// Lo que no se encontro en internet se vuelve a buscar pasado este tiempo.
const REINTENTO_DE_LO_NO_ENCONTRADO: Duration = Duration::from_secs(30 * 24 * 3_600);
const RESULTADOS: &str = "10";
const DEEZER_ALBUMES: &str = "https://api.deezer.com/search/album";
const DEEZER_TEMAS: &str = "https://api.deezer.com/search/track";
const ITUNES: &str = "https://itunes.apple.com/search";
/// iTunes nombra el tamano en la direccion de la imagen: se pide una mayor.
const ITUNES_MINIATURA: &str = "100x100bb";
const ITUNES_GRANDE: &str = "600x600bb";
const MUSICBRAINZ: &str = "https://musicbrainz.org/ws/2/release-group";
/// Puntaje minimo (0 a 100) de MusicBrainz para confiar en el album.
const PUNTAJE_MINIMO: u64 = 90;
const COVER_ART_ARCHIVE: &str = "https://coverartarchive.org/release-group";
/// MusicBrainz y el Cover Art Archive piden que el programa se identifique.
pub(super) const IDENTIFICACION: &str = concat!("MCPRaptorPlayer/", env!("CARGO_PKG_VERSION"), " ( jxlfybcrk@mozmail.com )");
/// Nombres (sin extension, en minusculas) de la imagen de un album en su
/// carpeta, en orden de preferencia.
const NOMBRES_DE_CARATULA: [&str; 8] = ["cover", "folder", "front", "portada", "caratula", "carátula", "album", "albumart"];
const EXTENSIONES_DE_IMAGEN: [&str; 3] = ["jpg", "jpeg", "png"];
/// Subcarpetas del album donde se suelen guardar las tapas escaneadas.
const SUBCARPETAS_DE_ARTE: [&str; 6] = ["covers", "cover", "scans", "artwork", "art", "caratulas"];
/// Subcarpeta de la carpeta de datos donde se guardan las de internet.
pub const CARPETA_DE_CARATULAS: &str = "caratulas";

/// De donde salio una carátula.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrigenDeLaCaratula {
    Archivo,
    Carpeta,
    Deezer,
    Itunes,
    MusicBrainz,
}

impl OrigenDeLaCaratula {
    pub fn describir(self) -> String {
        match self {
            Self::Archivo => tr!("Carátula: embebida en el archivo"),
            Self::Carpeta => tr!("Carátula: la imagen de la carpeta"),
            Self::Deezer => tr!("Carátula: Deezer"),
            Self::Itunes => tr!("Carátula: iTunes"),
            Self::MusicBrainz => tr!("Carátula: MusicBrainz (Cover Art Archive)"),
        }
    }

    fn codigo(self) -> &'static str {
        match self {
            Self::Archivo => "archivo",
            Self::Carpeta => "carpeta",
            Self::Deezer => "deezer",
            Self::Itunes => "itunes",
            Self::MusicBrainz => "musicbrainz",
        }
    }

    fn desde_codigo(codigo: &str) -> Option<Self> {
        [Self::Archivo, Self::Carpeta, Self::Deezer, Self::Itunes, Self::MusicBrainz].into_iter().find(|origen| origen.codigo() == codigo)
    }
}

/// Pixeles RGBA de `ancho` x `alto`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixeles {
    pub ancho: u32,
    pub alto: u32,
    pub rgba: Vec<u8>,
}

impl Pixeles {
    /// La imagen achicada a `lado` como maximo, sin deformarla (una chica
    /// no se agranda).
    fn de(imagen: &image::DynamicImage, lado: u32) -> Self {
        let rgba = if imagen.width() > lado || imagen.height() > lado { imagen.thumbnail(lado, lado).to_rgba8() } else { imagen.to_rgba8() };
        Self { ancho: rgba.width(), alto: rgba.height(), rgba: rgba.into_raw() }
    }
}

/// Una carátula lista para mostrar: la miniatura del panel, la ampliada
/// del globo y de donde salio.
#[derive(Clone, Debug, PartialEq)]
pub struct Miniatura {
    pub chica: Pixeles,
    pub ampliada: Pixeles,
    pub origen: OrigenDeLaCaratula,
}

/// La carátula que el tema tiene en su equipo: la tapa que trae el archivo,
/// la imagen de su carpeta o, si no hay, cualquier otra imagen del archivo
/// (que puede ser la contratapa o una pagina del librillo). None si no tiene
/// o no se puede leer.
pub fn caratula_local(archivo: &Path) -> Option<Miniatura> {
    let embebida = Decodificador::imagen_embebida(archivo).unwrap_or_else(|error| {
        log::debug!("Sin imagen en las etiquetas de {}: {error}", archivo.display());
        None
    });
    let (tapa, otra) = match embebida {
        Some((datos, true)) => (Some(datos), None),
        Some((datos, false)) => (None, Some(datos)),
        None => (None, None),
    };
    tapa.and_then(|datos| miniatura(&datos, OrigenDeLaCaratula::Archivo))
        .or_else(|| {
            let imagen = imagen_de_la_carpeta(archivo)?;
            miniatura(&fs::read(&imagen).ok()?, OrigenDeLaCaratula::Carpeta)
        })
        .or_else(|| otra.and_then(|datos| miniatura(&datos, OrigenDeLaCaratula::Archivo)))
}

/// Achica la imagen al lado de la miniatura, sin deformarla. None si no es
/// una imagen JPEG o PNG legible.
pub fn miniatura(datos: &[u8], origen: OrigenDeLaCaratula) -> Option<Miniatura> {
    let imagen = image::load_from_memory(datos).inspect_err(|error| log::debug!("Carátula ilegible: {error}")).ok()?;
    // La ampliada primero y de ella la chica: achicar la original grande dos
    // veces cuesta el doble.
    let ampliada = Pixeles::de(&imagen, LADO_DE_LA_AMPLIADA);
    let chica = if ampliada.ancho > LADO_DE_LA_MINIATURA || ampliada.alto > LADO_DE_LA_MINIATURA {
        let base = image::RgbaImage::from_raw(ampliada.ancho, ampliada.alto, ampliada.rgba.clone()).map(image::DynamicImage::ImageRgba8)?;
        Pixeles::de(&base, LADO_DE_LA_MINIATURA)
    } else {
        ampliada.clone()
    };
    Some(Miniatura { chica, ampliada, origen })
}

/// La imagen del album en la carpeta del tema o, si esa es una carpeta de
/// disco ("CD1", "Disco 2", "CD1 - Parte uno"), en la de arriba.
pub fn imagen_de_la_carpeta(archivo: &Path) -> Option<PathBuf> {
    let carpeta = archivo.parent()?;
    imagen_del_album(carpeta).or_else(|| {
        let nombre = carpeta.file_name()?.to_string_lossy();
        if es_carpeta_de_disco(&nombre) { imagen_del_album(carpeta.parent()?) } else { None }
    })
}

/// La imagen de la carpeta o, si no tiene, la de su subcarpeta de arte
/// ("Covers", "Scans", "Artwork"), donde muchos guardan las tapas escaneadas.
fn imagen_del_album(carpeta: &Path) -> Option<PathBuf> {
    imagen_en(carpeta).or_else(|| {
        fs::read_dir(carpeta)
            .ok()?
            .filter_map(Result::ok)
            .map(|entrada| entrada.path())
            .filter(|ruta| ruta.is_dir())
            .filter(|ruta| {
                ruta.file_name().is_some_and(|nombre| {
                    let nombre = nombre.to_string_lossy().to_lowercase();
                    SUBCARPETAS_DE_ARTE.contains(&nombre.as_str())
                })
            })
            .find_map(|subcarpeta| imagen_en(&subcarpeta))
    })
}

/// "CD1", "cd 2", "Disc_3", "Disco 1 - Lado A": el prefijo, un numero y,
/// si sigue algo, separado del numero (no "CDs" ni "Discografia").
fn es_carpeta_de_disco(nombre: &str) -> bool {
    let nombre = nombre.to_lowercase();
    ["disco", "disc", "disk", "cd"].iter().any(|prefijo| {
        nombre.strip_prefix(prefijo).is_some_and(|resto| {
            let resto = resto.trim_start_matches([' ', '_', '-', '.']);
            let cifras = resto.chars().take_while(char::is_ascii_digit).count();
            cifras > 0 && resto[cifras..].chars().next().is_none_or(|letra| !letra.is_alphanumeric())
        })
    })
}

/// La imagen de album de una carpeta, por el primero de los nombres
/// conocidos que aparezca (sin importar mayusculas). Tambien vale la del
/// Reproductor de Windows Media ("AlbumArt_{...}_Large.jpg").
fn imagen_en(carpeta: &Path) -> Option<PathBuf> {
    let imagenes: Vec<(String, String, PathBuf)> = fs::read_dir(carpeta)
        .ok()?
        .filter_map(Result::ok)
        .map(|entrada| entrada.path())
        .filter_map(|ruta| {
            let extension = ruta.extension()?.to_string_lossy().to_lowercase();
            let nombre = ruta.file_stem()?.to_string_lossy().to_lowercase();
            EXTENSIONES_DE_IMAGEN.contains(&extension.as_str()).then_some((nombre, extension, ruta))
        })
        .collect();
    let por_nombre = NOMBRES_DE_CARATULA.iter().find_map(|buscado| {
        EXTENSIONES_DE_IMAGEN
            .iter()
            .find_map(|extension| imagenes.iter().find(|(nombre, otra, _)| nombre == buscado && otra == extension))
    });
    por_nombre
        .or_else(|| imagenes.iter().find(|(nombre, _, _)| nombre.starts_with("albumart_") && nombre.ends_with("_large")))
        .map(|(_, _, ruta)| ruta.clone())
}

/// Lo que se busca en internet: por el album (todos los temas de un album
/// comparten la busqueda) o, si no se sabe, por el titulo del tema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsultaDeCaratula {
    artista: String,
    /// El album o el titulo sin lo que va entre parentesis, para buscar.
    nombre: String,
    /// El album o el titulo tal como esta, para comparar la edicion.
    buscado: String,
    por_album: bool,
}

impl ConsultaDeCaratula {
    /// None si falta el artista o no hay album ni titulo.
    pub fn nueva(artista: Option<&str>, album: Option<&str>, titulo: Option<&str>) -> Option<Self> {
        let artista = sin_parentesis(artista?.split([',', ';', '&']).next()?);
        fn limpio(texto: Option<&str>) -> Option<&str> {
            texto.map(str::trim).filter(|texto| !sin_parentesis(texto).is_empty())
        }
        let (buscado, por_album) = match (limpio(album), limpio(titulo)) {
            (Some(album), _) => (album, true),
            (None, Some(titulo)) => (titulo, false),
            (None, None) => return None,
        };
        (!artista.is_empty()).then(|| Self { artista, nombre: sin_parentesis(buscado), buscado: buscado.to_string(), por_album })
    }

    /// La misma consulta con el artista y el nombre saneados ("05. THE
    /// CURE" busca "THE CURE"); None si sanearlos no cambia nada.
    fn saneada(&self) -> Option<Self> {
        let artista = saneado(&self.artista);
        let buscado = saneado(&self.buscado);
        if artista.is_none() && buscado.is_none() {
            return None;
        }
        let buscado = buscado.unwrap_or_else(|| self.buscado.clone());
        Some(Self { artista: artista.unwrap_or_else(|| self.artista.clone()), nombre: sin_parentesis(&buscado), buscado, por_album: self.por_album })
    }

    /// Nombre de archivo estable para guardar el resultado.
    fn clave(&self) -> String {
        let texto = format!("{}|{}|{}", comparable(&self.artista), comparable(&self.buscado), self.por_album);
        // FNV-1a de 64 bits: estable entre versiones (a diferencia del hash de la biblioteca estandar).
        let hash = texto.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3));
        format!("{hash:016x}")
    }
}

/// Si la busqueda de esa carátula ya esta resuelta en `cache`: Some(true)
/// si se descargo, Some(false) si no se encontro hace menos de un mes (no se
/// vuelve a buscar todavia) y None si hay que salir a internet.
pub fn caratula_en_cache(consulta: &ConsultaDeCaratula, cache: &Path) -> Option<bool> {
    let base = cache.join(consulta.clave());
    if base.with_extension("img").is_file() {
        return Some(true);
    }
    fs::metadata(base.with_extension("ninguna"))
        .and_then(|datos| datos.modified())
        .is_ok_and(|momento| SystemTime::now().duration_since(momento).is_ok_and(|pasado| pasado < REINTENTO_DE_LO_NO_ENCONTRADO))
        .then_some(false)
}

/// Busca la carátula en internet, o la toma de lo ya guardado en `cache`.
/// Ok(None) si ninguna fuente tiene una confiable; Err solo si no respondio
/// ninguna (sin conexion), y en ese caso no se recuerda nada.
pub fn caratula_de_internet(consulta: &ConsultaDeCaratula, cache: &Path) -> Result<Option<Miniatura>, String> {
    let base = cache.join(consulta.clave());
    let (imagen, sin_resultado) = (base.with_extension("img"), base.with_extension("ninguna"));
    if let Ok(datos) = fs::read(&imagen) {
        let origen = fs::read_to_string(base.with_extension("origen"))
            .ok()
            .and_then(|codigo| OrigenDeLaCaratula::desde_codigo(codigo.trim()))
            .unwrap_or(OrigenDeLaCaratula::Deezer);
        return Ok(miniatura(&datos, origen));
    }
    let reciente = fs::metadata(&sin_resultado)
        .and_then(|datos| datos.modified())
        .is_ok_and(|momento| SystemTime::now().duration_since(momento).is_ok_and(|pasado| pasado < REINTENTO_DE_LO_NO_ENCONTRADO));
    if reciente {
        return Ok(None);
    }
    // Tal cual y, si no aparece, con el artista y el nombre saneados.
    let encontrada = match buscar_en_internet(consulta)? {
        None => consulta.saneada().map(|saneada| buscar_en_internet(&saneada)).transpose()?.flatten(),
        encontrada => encontrada,
    };
    let _ = fs::create_dir_all(cache);
    let Some((datos, origen)) = encontrada else {
        let _ = fs::write(&sin_resultado, b"");
        return Ok(None);
    };
    let Some(lista) = miniatura(&datos, origen) else {
        let _ = fs::write(&sin_resultado, b"");
        return Ok(None);
    };
    if let Err(error) = fs::write(&imagen, &datos).and_then(|()| fs::write(base.with_extension("origen"), origen.codigo())) {
        log::warn!("No se pudo guardar la carátula en {}: {error}", imagen.display());
    }
    Ok(Some(lista))
}

/// Pregunta a cada fuente en orden y descarga la primera carátula confiable.
fn buscar_en_internet(consulta: &ConsultaDeCaratula) -> Result<Option<(Vec<u8>, OrigenDeLaCaratula)>, String> {
    let agente: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(TIEMPO_MAXIMO_DE_LA_CONSULTA)).user_agent(IDENTIFICACION).build().into();
    let mut respondio = false;
    let mut ultimo_error = None;
    for origen in [OrigenDeLaCaratula::Deezer, OrigenDeLaCaratula::Itunes, OrigenDeLaCaratula::MusicBrainz] {
        let direccion = match origen {
            OrigenDeLaCaratula::Deezer => buscar_en_deezer(&agente, consulta),
            OrigenDeLaCaratula::Itunes => buscar_en_itunes(&agente, consulta),
            // MusicBrainz solo conoce albumes.
            OrigenDeLaCaratula::MusicBrainz if consulta.por_album => buscar_en_musicbrainz(&agente, consulta),
            _ => continue,
        };
        let descarga = direccion.and_then(|direccion| direccion.map(|direccion| descargar(&agente, &direccion)).transpose());
        match descarga {
            Ok(Some(Some(datos))) => return Ok(Some((datos, origen))),
            Ok(_) => respondio = true,
            Err(error) => {
                log::debug!("Carátula: {origen:?} no respondio: {error}");
                ultimo_error = Some(error);
            }
        }
    }
    match (respondio, ultimo_error) {
        (false, Some(error)) => Err(error),
        _ => Ok(None),
    }
}

/// Una consulta GET en JSON. Si el servicio esta ocupado (503, como responde
/// MusicBrainz a quien pregunta seguido), se reintenta una vez tras una pausa.
pub(super) fn pedir_json(agente: &ureq::Agent, direccion: &str, parametros: &[(&str, &str)]) -> Result<Value, String> {
    let consultar = || {
        let mut pedido = agente.get(direccion);
        for (clave, valor) in parametros {
            pedido = pedido.query(*clave, *valor);
        }
        pedido.call()
    };
    let respuesta = match consultar() {
        Err(ureq::Error::StatusCode(503)) => {
            std::thread::sleep(super::PAUSA_ENTRE_CONSULTAS * 2);
            consultar()
        }
        otra => otra,
    };
    let texto = respuesta
        .map_err(|error| tr!("No se pudo consultar {direccion}: {error}", direccion = direccion, error = error))?
        .body_mut()
        .read_to_string()
        .map_err(|error| tr!("No se pudo leer la respuesta de {direccion}: {error}", direccion = direccion, error = error))?;
    serde_json::from_str(&texto).map_err(|error| tr!("Respuesta ilegible de {direccion}: {error}", direccion = direccion, error = error))
}

/// Los bytes de la imagen; None si la fuente dice que no la tiene (404).
fn descargar(agente: &ureq::Agent, direccion: &str) -> Result<Option<Vec<u8>>, String> {
    let respuesta = match agente.get(direccion).call() {
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        otra => otra.map_err(|error| tr!("No se pudo descargar la carátula: {error}", error = error))?,
    };
    let mut datos = Vec::new();
    respuesta
        .into_body()
        .into_reader()
        .take(TAMANO_MAXIMO_DE_LA_IMAGEN + 1)
        .read_to_end(&mut datos)
        .map_err(|error| tr!("No se pudo descargar la carátula: {error}", error = error))?;
    Ok((!datos.is_empty() && datos.len() as u64 <= TAMANO_MAXIMO_DE_LA_IMAGEN).then_some(datos))
}

fn buscar_en_deezer(agente: &ureq::Agent, consulta: &ConsultaDeCaratula) -> Result<Option<String>, String> {
    let terminos = format!("{} {}", consulta.artista, consulta.nombre);
    let direccion = if consulta.por_album { DEEZER_ALBUMES } else { DEEZER_TEMAS };
    let respuesta = pedir_json(agente, direccion, &[("q", terminos.as_str()), ("limit", RESULTADOS)])?;
    // Deezer avisa en el cuerpo, con estado 200, cuando rechaza la busqueda.
    if let Some(error) = respuesta.get("error") {
        return Err(tr!("Deezer rechazo la busqueda: {error}", error = error));
    }
    Ok(elegir_de_deezer(&respuesta, consulta))
}

fn elegir_de_deezer(respuesta: &Value, consulta: &ConsultaDeCaratula) -> Option<String> {
    let candidatos = respuesta.get("data")?.as_array()?.iter().filter_map(|resultado| {
        let artista = resultado.get("artist")?.get("name")?.as_str()?;
        let imagen = if consulta.por_album { resultado.get("cover_xl") } else { resultado.get("album")?.get("cover_xl") };
        if !mismo_artista(artista, &consulta.artista) {
            return None;
        }
        Some((resultado.get("title")?.as_str()?.to_string(), imagen?.as_str()?.to_string()))
    });
    elegir(candidatos, consulta)
}

fn buscar_en_itunes(agente: &ureq::Agent, consulta: &ConsultaDeCaratula) -> Result<Option<String>, String> {
    let terminos = format!("{} {}", consulta.artista, consulta.nombre);
    let entidad = if consulta.por_album { "album" } else { "song" };
    let respuesta = pedir_json(agente, ITUNES, &[("term", terminos.as_str()), ("entity", entidad), ("limit", RESULTADOS)])?;
    Ok(elegir_de_itunes(&respuesta, consulta))
}

fn elegir_de_itunes(respuesta: &Value, consulta: &ConsultaDeCaratula) -> Option<String> {
    let candidatos = respuesta.get("results")?.as_array()?.iter().filter_map(|resultado| {
        let artista = resultado.get("artistName")?.as_str()?;
        let nombre = resultado.get(if consulta.por_album { "collectionName" } else { "trackName" })?.as_str()?;
        let imagen = resultado.get("artworkUrl100")?.as_str()?.replace(ITUNES_MINIATURA, ITUNES_GRANDE);
        mismo_artista(artista, &consulta.artista).then(|| (nombre.to_string(), imagen))
    });
    elegir(candidatos, consulta)
}

fn buscar_en_musicbrainz(agente: &ureq::Agent, consulta: &ConsultaDeCaratula) -> Result<Option<String>, String> {
    let frase = |texto: &str| format!("\"{}\"", texto.replace('\\', "\\\\").replace('"', "\\\""));
    let terminos = format!("releasegroup:{} AND artist:{}", frase(&consulta.nombre), frase(&consulta.artista));
    let respuesta = pedir_json(agente, MUSICBRAINZ, &[("query", terminos.as_str()), ("fmt", "json"), ("limit", "5")])?;
    Ok(elegir_de_musicbrainz(&respuesta, consulta))
}

fn elegir_de_musicbrainz(respuesta: &Value, consulta: &ConsultaDeCaratula) -> Option<String> {
    let candidatos = respuesta.get("release-groups")?.as_array()?.iter().filter_map(|grupo| {
        let puntaje = grupo.get("score").and_then(Value::as_u64).unwrap_or(0);
        let artista = grupo.get("artist-credit")?.as_array()?.first()?.get("name")?.as_str()?;
        (puntaje >= PUNTAJE_MINIMO && mismo_artista(artista, &consulta.artista)).then(|| {
            let id = grupo.get("id")?.as_str()?;
            Some((grupo.get("title")?.as_str()?.to_string(), format!("{COVER_ART_ARCHIVE}/{id}/front-500")))
        })?
    });
    elegir(candidatos, consulta)
}

/// De los candidatos (nombre, imagen) del mismo artista, el de nombre igual
/// al buscado o, si no hay, el primero que solo le agrega palabras al final
/// ("Álbum - EP"). Nunca uno de otra edicion: si los parentesis de uno u
/// otro nombran algo que el otro no ("Deluxe", "En vivo"), puede tener otra
/// tapa. Solo direcciones https.
fn elegir(candidatos: impl Iterator<Item = (String, String)>, consulta: &ConsultaDeCaratula) -> Option<String> {
    let buscado = comparable(&sin_parentesis(&consulta.buscado));
    let palabras_buscadas = comparable(&consulta.buscado);
    let palabras_buscadas: Vec<&str> = palabras_buscadas.split(' ').collect();
    let edicion_buscada = if consulta.por_album { entre_parentesis(&consulta.buscado) } else { Vec::new() };
    let mut parecido = None;
    for (nombre, imagen) in candidatos {
        if !imagen.starts_with("https://") {
            continue;
        }
        let palabras = comparable(&nombre);
        let palabras: Vec<&str> = palabras.split(' ').collect();
        let otra_edicion = entre_parentesis(&nombre).iter().any(|palabra| !palabras_buscadas.contains(&palabra.as_str()))
            || edicion_buscada.iter().any(|palabra| !palabras.contains(&palabra.as_str()));
        if otra_edicion {
            continue;
        }
        let nucleo = comparable(&sin_parentesis(&nombre));
        if nucleo == buscado {
            return Some(imagen);
        }
        if parecido.is_none() && agrega_palabras(&nucleo, &buscado) {
            parecido = Some(imagen);
        }
    }
    parecido
}

pub(super) fn mismo_artista(encontrado: &str, buscado: &str) -> bool {
    let (encontrado, buscado) = (comparable(encontrado), comparable(buscado));
    encontrado == buscado || agrega_palabras(&encontrado, &buscado)
}

/// Si uno de los dos nombres es el otro con palabras enteras agregadas al final.
pub(super) fn agrega_palabras(uno: &str, otro: &str) -> bool {
    let empieza = |largo: &str, corto: &str| !corto.is_empty() && largo.strip_prefix(corto).is_some_and(|resto| resto.starts_with(' '));
    empieza(uno, otro) || empieza(otro, uno)
}

/// El texto sin lo que va entre parentesis o corchetes, que suele impedir que
/// la busqueda encuentre el album ("[Remasterizado]").
pub(super) fn sin_parentesis(texto: &str) -> String {
    let mut queda = String::with_capacity(texto.len());
    let mut profundidad = 0usize;
    for letra in texto.chars() {
        match letra {
            '(' | '[' => profundidad += 1,
            ')' | ']' => profundidad = profundidad.saturating_sub(1),
            _ if profundidad == 0 => queda.push(letra),
            _ => {}
        }
    }
    queda.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Las palabras entre parentesis o corchetes, comparables.
fn entre_parentesis(texto: &str) -> Vec<String> {
    let mut adentro = String::new();
    let mut profundidad = 0usize;
    for letra in texto.chars() {
        match letra {
            '(' | '[' => {
                profundidad += 1;
                adentro.push(' ');
            }
            ')' | ']' => profundidad = profundidad.saturating_sub(1),
            _ if profundidad > 0 => adentro.push(letra),
            _ => {}
        }
    }
    comparable(&adentro).split(' ').filter(|palabra| !palabra.is_empty()).map(str::to_string).collect()
}

/// El texto para una segunda busqueda, cuando la literal no encuentra
/// nada: sin el numero de pista del principio ("05. THE CURE", "01 - Tema",
/// "1-01 Tema") ni signos sueltos ("Artista_-_Tema"). None si queda igual o
/// vacio. No reemplaza la busqueda literal: "99 Luftballons" o "1979" se
/// buscan primero tal cual.
pub(super) fn saneado(texto: &str) -> Option<String> {
    let recortado = texto.trim();
    let sin_numero = sin_numero_de_pista(recortado);
    let limpio = sin_numero
        .chars()
        .map(|letra| if letra.is_alphanumeric() || matches!(letra, '\'' | '’' | '&' | '(' | ')' | '[' | ']') { letra } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!limpio.is_empty() && limpio != recortado).then_some(limpio)
}

/// El texto sin el numero de pista del principio: de 1 a 3 cifras (o un
/// disco y pista, "1-01") seguidas de un separador. Si despues no queda
/// nada, el texto entero.
fn sin_numero_de_pista(texto: &str) -> &str {
    let cifras = |texto: &str| texto.chars().take_while(char::is_ascii_digit).count();
    let primeras = cifras(texto);
    if primeras == 0 || primeras > 3 {
        return texto;
    }
    let mut resto = &texto[primeras..];
    // "1-01 Tema": el disco y la pista.
    if let Some(despues) = resto.strip_prefix('-') {
        let segundas = cifras(despues);
        if (1..=3).contains(&segundas) {
            resto = &despues[segundas..];
        }
    }
    let sin_separador = resto.trim_start_matches(['.', '-', '_', ')', ' ', '\t']);
    if sin_separador.len() == resto.len() || sin_separador.is_empty() { texto } else { sin_separador }
}

/// En minusculas, sin acentos y con cada tramo de signos y espacios como un
/// solo espacio.
pub(super) fn comparable(texto: &str) -> String {
    crate::dominio::simplificar_nombre(texto)
        .split(|letra: char| !letra.is_alphanumeric())
        .filter(|palabra| !palabra.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn consulta(artista: &str, album: Option<&str>, titulo: Option<&str>) -> ConsultaDeCaratula {
        ConsultaDeCaratula::nueva(Some(artista), album, titulo).unwrap()
    }

    fn carpeta_temporal(nombre: &str) -> PathBuf {
        let carpeta = std::env::temp_dir().join(format!("raptor-pruebas-caratulas-{nombre}"));
        let _ = fs::remove_dir_all(&carpeta);
        fs::create_dir_all(&carpeta).unwrap();
        carpeta
    }

    /// Un PNG valido de `lado` x `lado`.
    fn png(lado: u32) -> Vec<u8> {
        let imagen = image::RgbaImage::from_pixel(lado, lado, image::Rgba([200, 30, 30, 255]));
        let mut datos = std::io::Cursor::new(Vec::new());
        imagen.write_to(&mut datos, image::ImageFormat::Png).unwrap();
        datos.into_inner()
    }

    #[test]
    fn sanea_el_numero_de_pista_y_los_signos_para_una_segunda_busqueda() {
        assert_eq!(saneado("05. THE CURE").as_deref(), Some("THE CURE"));
        assert_eq!(saneado("01 - Down on the Street").as_deref(), Some("Down on the Street"));
        assert_eq!(saneado("1-01 Riot Radio").as_deref(), Some("Riot Radio"));
        assert_eq!(saneado("07_Kundalini_Express").as_deref(), Some("Kundalini Express"));
        assert_eq!(saneado("Soda Stereo - (En vivo)").as_deref(), Some("Soda Stereo (En vivo)"));
        assert_eq!(saneado("Guns N' Roses"), None, "el apostrofo queda");
        assert_eq!(saneado("The Cure"), None, "sin cambios no hay segunda busqueda");
        assert_eq!(saneado("1979"), None, "cuatro cifras no son un numero de pista");
        assert_eq!(saneado("99 Luftballons").as_deref(), Some("Luftballons"), "se busca primero tal cual");
        assert_eq!(saneado("05.").as_deref(), Some("05"), "sin nada despues, el numero queda");
        let consulta = ConsultaDeCaratula::nueva(Some("05. THE CURE"), Some("Disintegration"), None).unwrap();
        let saneada = consulta.saneada().unwrap();
        assert_eq!((saneada.artista.as_str(), saneada.nombre.as_str()), ("THE CURE", "Disintegration"));
        assert!(ConsultaDeCaratula::nueva(Some("The Cure"), Some("Disintegration"), None).unwrap().saneada().is_none());
    }

    #[test]
    fn la_consulta_va_por_album_y_si_no_por_titulo() {
        let por_album = consulta("Soda Stereo, Andrea Echeverri", Some("Comfort y música para volar (En vivo)"), Some("Té para 3"));
        assert!(por_album.por_album);
        assert_eq!(por_album.artista, "Soda Stereo");
        assert_eq!(por_album.nombre, "Comfort y música para volar");
        let por_titulo = consulta("Queen", Some("  "), Some("Bohemian Rhapsody [Remastered]"));
        assert!(!por_titulo.por_album);
        assert_eq!(por_titulo.nombre, "Bohemian Rhapsody");
        assert!(ConsultaDeCaratula::nueva(None, Some("Album"), None).is_none());
        assert!(ConsultaDeCaratula::nueva(Some("Queen"), None, None).is_none());
        // La clave no depende de mayusculas ni acentos.
        assert_eq!(consulta("QUEEN", Some("Innuendo"), None).clave(), consulta("Queen", Some("innuendo"), None).clave());
        assert_ne!(consulta("Queen", Some("Innuendo"), None).clave(), consulta("Queen", Some("Jazz"), None).clave());
    }

    #[test]
    fn solo_acepta_el_mismo_artista_y_la_misma_edicion() {
        let buscada = consulta("Queen", Some("A Night at the Opera"), None);
        let candidatos = |lista: &[(&str, &str)]| lista.iter().map(|(nombre, imagen)| (nombre.to_string(), imagen.to_string())).collect::<Vec<_>>();
        // La exacta le gana a la que agrega palabras, aunque venga despues.
        let lista = candidatos(&[
            ("A Night at the Opera (Deluxe Edition)", "https://x/deluxe"),
            ("A Night at the Opera - Remastered", "https://x/agrega"),
            ("A Night At The Opera", "https://x/exacta"),
        ]);
        assert_eq!(elegir(lista.into_iter(), &buscada).as_deref(), Some("https://x/exacta"));
        let lista = candidatos(&[("A Night at the Opera (Deluxe Edition)", "https://x/deluxe"), ("A Night at the Opera - EP", "https://x/ep")]);
        assert_eq!(elegir(lista.into_iter(), &buscada).as_deref(), Some("https://x/ep"));
        // La version Deluxe no reemplaza a la comun ni al reves; tampoco una direccion sin https.
        let lista = candidatos(&[("A Night at the Opera (Deluxe Edition)", "https://x/deluxe"), ("A Night at the Opera", "http://x/inseguro")]);
        assert_eq!(elegir(lista.into_iter(), &buscada), None);
        let deluxe = consulta("Queen", Some("A Night at the Opera (Deluxe Edition)"), None);
        let lista = candidatos(&[("A Night at the Opera", "https://x/comun"), ("A Night at the Opera (Deluxe Edition)", "https://x/deluxe")]);
        assert_eq!(elegir(lista.into_iter(), &deluxe).as_deref(), Some("https://x/deluxe"));
        assert!(mismo_artista("Queen & David Bowie", "Queen") && mismo_artista("Charly García", "charly garcia"));
        assert!(!mismo_artista("Queensryche", "Queen"));
    }

    #[test]
    fn lee_las_respuestas_de_cada_fuente() {
        let album = consulta("Soda Stereo", Some("Canción Animal"), None);
        let deezer: Value = serde_json::from_str(
            r#"{"data": [
                {"title": "Canción Animal", "artist": {"name": "Otro"}, "cover_xl": "https://deezer/otro"},
                {"title": "Cancion Animal", "artist": {"name": "Soda Stereo"}, "cover_xl": "https://deezer/bien"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(elegir_de_deezer(&deezer, &album).as_deref(), Some("https://deezer/bien"));
        let itunes: Value = serde_json::from_str(
            r#"{"results": [{"artistName": "Soda Stereo", "collectionName": "Canción Animal", "artworkUrl100": "https://itunes/a/100x100bb.jpg"}]}"#,
        )
        .unwrap();
        assert_eq!(elegir_de_itunes(&itunes, &album).as_deref(), Some("https://itunes/a/600x600bb.jpg"));
        let musicbrainz: Value = serde_json::from_str(
            r#"{"release-groups": [
                {"id": "dudoso", "score": 70, "title": "Canción Animal", "artist-credit": [{"name": "Soda Stereo"}]},
                {"id": "bueno", "score": 100, "title": "Canción animal", "artist-credit": [{"name": "Soda Stereo"}]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            elegir_de_musicbrainz(&musicbrainz, &album).as_deref(),
            Some("https://coverartarchive.org/release-group/bueno/front-500")
        );
        // Por titulo, Deezer da la tapa del album del tema.
        let tema = consulta("Soda Stereo", None, Some("Un millón de años luz"));
        let temas: Value = serde_json::from_str(
            r#"{"data": [{"title": "Un Millón de Años Luz", "artist": {"name": "Soda Stereo"}, "album": {"cover_xl": "https://deezer/tema"}}]}"#,
        )
        .unwrap();
        assert_eq!(elegir_de_deezer(&temas, &tema).as_deref(), Some("https://deezer/tema"));
    }

    #[test]
    fn toma_la_imagen_de_la_carpeta_o_la_del_album_sobre_el_disco() {
        let carpeta = carpeta_temporal("carpeta");
        let disco = carpeta.join("CD 2");
        fs::create_dir_all(&disco).unwrap();
        fs::write(carpeta.join("tema.mp3"), b"").unwrap();
        fs::write(disco.join("tema.mp3"), b"").unwrap();
        assert_eq!(imagen_de_la_carpeta(&carpeta.join("tema.mp3")), None);
        fs::write(carpeta.join("otra.jpg"), png(4)).unwrap();
        fs::write(carpeta.join("Folder.JPG"), png(4)).unwrap();
        fs::write(carpeta.join("AlbumArt_{X}_Large.jpg"), png(4)).unwrap();
        assert_eq!(imagen_de_la_carpeta(&carpeta.join("tema.mp3")), Some(carpeta.join("Folder.JPG")));
        fs::write(carpeta.join("cover.png"), png(4)).unwrap();
        assert_eq!(imagen_de_la_carpeta(&carpeta.join("tema.mp3")), Some(carpeta.join("cover.png")), "cover va antes que folder");
        assert_eq!(imagen_de_la_carpeta(&disco.join("tema.mp3")), Some(carpeta.join("cover.png")), "la del album sobre el disco");
        for nombre in ["CD1", "cd 2", "Disc_3", "Disco 1 - Lado A", "CD1 - Original Album Remastered Part One"] {
            assert!(es_carpeta_de_disco(nombre), "{nombre}");
        }
        for nombre in ["CDs", "Discografia", "CD", "Disco", "CD1A", "Rock"] {
            assert!(!es_carpeta_de_disco(nombre), "{nombre}");
        }
        let suelta = carpeta_temporal("suelta");
        fs::write(suelta.join("AlbumArt_{X}_Large.jpg"), png(4)).unwrap();
        assert_eq!(imagen_de_la_carpeta(&suelta.join("tema.mp3")), Some(suelta.join("AlbumArt_{X}_Large.jpg")));
        // Un album con sus tapas en "Covers" y los temas en carpetas de disco.
        let caja = carpeta_temporal("caja");
        let (tapas, disco) = (caja.join("Covers"), caja.join("CD1 - Parte uno"));
        fs::create_dir_all(&tapas).unwrap();
        fs::create_dir_all(&disco).unwrap();
        fs::write(tapas.join("Box.jpg"), png(4)).unwrap();
        fs::write(tapas.join("Front.jpg"), png(4)).unwrap();
        assert_eq!(imagen_de_la_carpeta(&disco.join("tema.flac")), Some(tapas.join("Front.jpg")));
        let _ = fs::remove_dir_all(caja);
        let _ = fs::remove_dir_all(carpeta);
        let _ = fs::remove_dir_all(suelta);
    }

    #[test]
    fn la_miniatura_no_pasa_del_lado_y_conserva_la_proporcion() {
        let grande = image::RgbaImage::from_pixel(800, 400, image::Rgba([0, 0, 0, 255]));
        let mut datos = std::io::Cursor::new(Vec::new());
        grande.write_to(&mut datos, image::ImageFormat::Png).unwrap();
        let caratula = miniatura(datos.get_ref(), OrigenDeLaCaratula::Carpeta).unwrap();
        let (chica, ampliada) = (&caratula.chica, &caratula.ampliada);
        assert_eq!((chica.ancho, chica.alto), (LADO_DE_LA_MINIATURA, LADO_DE_LA_MINIATURA / 2));
        assert_eq!(chica.rgba.len(), (chica.ancho * chica.alto * 4) as usize);
        assert_eq!((ampliada.ancho, ampliada.alto), (LADO_DE_LA_AMPLIADA, LADO_DE_LA_AMPLIADA / 2));
        assert_eq!(ampliada.rgba.len(), (ampliada.ancho * ampliada.alto * 4) as usize);
        let pequena = miniatura(&png(20), OrigenDeLaCaratula::Archivo).unwrap();
        assert_eq!((pequena.chica.ancho, pequena.chica.alto), (20, 20), "una chica no se agranda");
        assert_eq!((pequena.ampliada.ancho, pequena.ampliada.alto), (20, 20));
        assert!(miniatura(b"no es una imagen", OrigenDeLaCaratula::Archivo).is_none());
    }

    #[test]
    fn lo_guardado_se_usa_sin_volver_a_preguntar() {
        let cache = carpeta_temporal("cache");
        let buscada = consulta("Queen", Some("Innuendo"), None);
        let base = cache.join(buscada.clave());
        fs::write(base.with_extension("img"), png(30)).unwrap();
        fs::write(base.with_extension("origen"), "itunes").unwrap();
        assert_eq!(caratula_en_cache(&buscada, &cache), Some(true));
        let guardada = caratula_de_internet(&buscada, &cache).unwrap().unwrap();
        assert_eq!(guardada.origen, OrigenDeLaCaratula::Itunes);
        // Lo que no se encontro hace poco no se vuelve a buscar (no sale a internet).
        let otra = consulta("Queen", Some("Jazz"), None);
        assert_eq!(caratula_en_cache(&otra, &cache), None, "sin buscar todavia");
        fs::write(cache.join(otra.clave()).with_extension("ninguna"), b"").unwrap();
        assert_eq!(caratula_en_cache(&otra, &cache), Some(false));
        assert_eq!(caratula_de_internet(&otra, &cache).unwrap(), None);
        let _ = fs::remove_dir_all(cache);
    }

    /// Busqueda real (necesita internet): se corre a mano con
    /// `cargo test caratula_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn caratula_real_en_internet() {
        for (artista, album) in [("Soda Stereo", "Canción Animal"), ("Queen", "A Night at the Opera")] {
            let encontrada = buscar_en_internet(&consulta(artista, Some(album), None)).unwrap();
            println!("{album}: {:?}", encontrada.as_ref().map(|(datos, origen)| (datos.len(), origen)));
            assert!(encontrada.is_some(), "{album}");
        }
        // Cada fuente por separado, para saber que las de respaldo responden.
        let agente: ureq::Agent = ureq::Agent::config_builder().user_agent(IDENTIFICACION).build().into();
        let buscada = consulta("Queen", Some("A Night at the Opera"), None);
        let itunes = buscar_en_itunes(&agente, &buscada).unwrap();
        println!("iTunes: {itunes:?}");
        assert!(itunes.is_some());
        // MusicBrainz limita a quien consulta seguido (503): entonces solo se informa.
        match buscar_en_musicbrainz(&agente, &buscada) {
            Ok(direccion) => {
                println!("MusicBrainz: {direccion:?}");
                let direccion = direccion.expect("MusicBrainz conoce el album");
                assert!(descargar(&agente, &direccion).unwrap().is_some(), "el Cover Art Archive redirige a la imagen");
            }
            Err(error) => println!("MusicBrainz no disponible ahora: {error}"),
        }
    }
}
