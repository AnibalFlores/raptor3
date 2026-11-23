//! Completar los datos de un tema (titulo, artista, album, año y genero)
//! desde internet, con la misma idea que las carátulas: varias fuentes en
//! orden, y cada una completa solo lo que las anteriores no trajeron.
//!
//! 1. MusicBrainz: la grabacion original y el album de estudio donde salio,
//!    el año de la primera edicion y el genero mas votado.
//! 2. Deezer: el tema y su album (año, genero y si es un recopilatorio).
//! 3. iTunes: album, año y genero del tema.
//!
//! De Deezer e iTunes solo se acepta un tema del mismo artista, con el mismo
//! titulo (o el mismo con palabras agregadas, como "- Remastered") y, si se
//! conoce, una duracion parecida. De un recopilatorio ("Grandes exitos",
//! "Greatest Hits") no se toman el album ni el año: serian los de la
//! recopilacion, no los del tema. Ninguna fuente pide cuenta ni clave.

use serde_json::Value;

use super::caratulas::{IDENTIFICACION, TIEMPO_MAXIMO_DE_LA_CONSULTA, agrega_palabras, comparable, mismo_artista, pedir_json, saneado, sin_parentesis};
use super::musicbrainz::buscar_grabacion;
use crate::dominio::{CampoDescriptivo, DatosDescriptivos};

const DEEZER_TEMAS: &str = "https://api.deezer.com/search/track";
const DEEZER_ALBUM: &str = "https://api.deezer.com/album";
const ITUNES: &str = "https://itunes.apple.com/search";
const RESULTADOS: &str = "10";
/// Diferencia maxima de duracion para aceptar el tema, si se conoce.
const TOLERANCIA_DE_DURACION: f64 = 8.0;
/// Palabras de una aclaracion que nombra la edicion de un album, no el album.
const DE_EDICION: [&str; 10] = ["deluxe", "remaster", "remastered", "remasterizado", "edition", "edicion", "expanded", "anniversary", "aniversario", "bonus"];
/// Palabras de un album que delatan un recopilatorio.
const DE_RECOPILATORIO: [&str; 19] = [
    "greatest hits",
    "greatest",
    "best of",
    "the best",
    "the very best",
    "hits",
    "ultimate",
    "singles",
    "collection",
    "collections",
    "anthology",
    "essential",
    "essentials",
    "grandes exitos",
    "lo mejor",
    "exitos",
    "coleccion",
    "antologia",
    "compilation",
];

/// De donde salio cada dato.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuenteDeDatos {
    MusicBrainz,
    Deezer,
    Itunes,
}

impl FuenteDeDatos {
    pub fn nombre(self) -> &'static str {
        match self {
            Self::MusicBrainz => "MusicBrainz",
            Self::Deezer => "Deezer",
            Self::Itunes => "iTunes",
        }
    }
}

/// Lo que se busca: lo que ya dice la ficha del tema.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TemaBuscado {
    pub titulo: String,
    pub artista: Option<String>,
    pub duracion: Option<f64>,
}

/// Los datos encontrados y la fuente de cada uno.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DatosDeInternet {
    pub datos: DatosDescriptivos,
    pub fuentes: Vec<(CampoDescriptivo, FuenteDeDatos)>,
}

impl DatosDeInternet {
    /// Suma lo que trae una fuente en los datos que todavia faltan.
    fn sumar(&mut self, nuevos: DatosDescriptivos, fuente: FuenteDeDatos) {
        let mut sumar_texto = |destino: &mut Option<String>, valor: Option<String>, campo| {
            if destino.is_none()
                && let Some(valor) = valor.map(|valor| valor.trim().to_string()).filter(|valor| !valor.is_empty())
            {
                *destino = Some(valor);
                self.fuentes.push((campo, fuente));
            }
        };
        let datos = &mut self.datos;
        sumar_texto(&mut datos.titulo, nuevos.titulo, CampoDescriptivo::Titulo);
        sumar_texto(&mut datos.artista, nuevos.artista, CampoDescriptivo::Artista);
        sumar_texto(&mut datos.album, nuevos.album, CampoDescriptivo::Album);
        sumar_texto(&mut datos.genero, nuevos.genero, CampoDescriptivo::Genero);
        if datos.anio.is_none()
            && let Some(anio) = nuevos.anio
        {
            datos.anio = Some(anio);
            self.fuentes.push((CampoDescriptivo::Anio, fuente));
        }
    }

    fn completo(&self) -> bool {
        CampoDescriptivo::TODOS.into_iter().all(|campo| self.datos.valor(campo).is_some())
    }

    fn vacio(&self) -> bool {
        self.fuentes.is_empty()
    }

    /// La fuente de un dato.
    #[cfg(test)]
    fn fuente_de(&self, campo: CampoDescriptivo) -> Option<FuenteDeDatos> {
        self.fuentes.iter().find(|(otro, _)| *otro == campo).map(|(_, fuente)| *fuente)
    }
}

impl TemaBuscado {
    /// El mismo tema con el titulo y el artista saneados ("05. THE CURE"
    /// busca "THE CURE"); None si sanearlos no cambia nada.
    fn saneado(&self) -> Option<Self> {
        let titulo = saneado(&self.titulo);
        let artista = self.artista.as_deref().and_then(saneado);
        if titulo.is_none() && artista.is_none() {
            return None;
        }
        Some(Self { titulo: titulo.unwrap_or_else(|| self.titulo.clone()), artista: artista.or_else(|| self.artista.clone()), duracion: self.duracion })
    }
}

/// Busca el tema tal cual y, si no aparece en ninguna fuente, con el titulo
/// y el artista saneados (sin numero de pista ni signos sueltos).
pub fn buscar_datos(tema: &TemaBuscado) -> Result<Option<DatosDeInternet>, String> {
    match buscar_datos_tal_cual(tema)? {
        None => match tema.saneado() {
            Some(saneado) => {
                log::debug!("Datos de internet: sin resultados para {:?}; se busca saneado como {:?}", tema.titulo, saneado.titulo);
                buscar_datos_tal_cual(&saneado)
            }
            None => Ok(None),
        },
        encontrado => Ok(encontrado),
    }
}

/// Busca el tema en cada fuente, en orden, hasta tener todos los datos.
/// Ok(None) si ninguna tiene una coincidencia segura; Err solo si no
/// respondio ninguna (sin conexion). Bloquea mientras consulta.
fn buscar_datos_tal_cual(tema: &TemaBuscado) -> Result<Option<DatosDeInternet>, String> {
    let mut encontrado = DatosDeInternet::default();
    let mut respondio = false;
    let mut ultimo_error = None;
    let mut anotar = |resultado: Result<Option<DatosDescriptivos>, String>, fuente, encontrado: &mut DatosDeInternet| match resultado {
        Ok(datos) => {
            respondio = true;
            if let Some(datos) = datos {
                encontrado.sumar(datos, fuente);
            }
        }
        Err(error) => {
            log::debug!("Datos de internet: {} no respondio: {error}", fuente.nombre());
            ultimo_error = Some(error);
        }
    };
    let grabacion = buscar_grabacion(&tema.titulo, tema.artista.as_deref(), tema.duracion).map(|coincidencia| coincidencia.map(|coincidencia| coincidencia.datos));
    anotar(grabacion, FuenteDeDatos::MusicBrainz, &mut encontrado);
    // Deezer e iTunes buscan por artista y titulo: sin artista (ni el de la
    // ficha ni el de MusicBrainz) no hay como confirmar que es el mismo tema.
    let artista = tema.artista.clone().or_else(|| encontrado.datos.artista.clone());
    if let Some(artista) = artista.filter(|_| !encontrado.completo()) {
        let buscado = TemaBuscado { artista: Some(artista), ..tema.clone() };
        let agente: ureq::Agent =
            ureq::Agent::config_builder().timeout_global(Some(TIEMPO_MAXIMO_DE_LA_CONSULTA)).user_agent(IDENTIFICACION).build().into();
        // El año de Deezer suele ser el de la edicion que tiene (una
        // reedicion); se guarda aparte para quedarse con el mas antiguo.
        let (deezer, anio_de_deezer) = separar_el_anio(buscar_en_deezer(&agente, &buscado));
        anotar(deezer, FuenteDeDatos::Deezer, &mut encontrado);
        let mut anio_de_itunes = None;
        if !encontrado.completo() {
            let itunes;
            (itunes, anio_de_itunes) = separar_el_anio(buscar_en_itunes(&agente, &buscado));
            anotar(itunes, FuenteDeDatos::Itunes, &mut encontrado);
        }
        let mas_antiguo = [(anio_de_deezer, FuenteDeDatos::Deezer), (anio_de_itunes, FuenteDeDatos::Itunes)]
            .into_iter()
            .filter_map(|(anio, fuente)| Some((anio?, fuente)))
            .min_by_key(|(anio, _)| *anio);
        if let Some((anio, fuente)) = mas_antiguo {
            encontrado.sumar(DatosDescriptivos { anio: Some(anio), ..DatosDescriptivos::default() }, fuente);
        }
    }
    match (encontrado.vacio(), respondio, ultimo_error) {
        (false, _, _) => Ok(Some(encontrado)),
        (true, false, Some(error)) => Err(error),
        _ => Ok(None),
    }
}

/// Saca el año de lo que trajo una fuente, para elegirlo aparte.
fn separar_el_anio(resultado: Result<Option<DatosDescriptivos>, String>) -> (Result<Option<DatosDescriptivos>, String>, Option<u16>) {
    match resultado {
        Ok(Some(mut datos)) => {
            let anio = datos.anio.take();
            (Ok(Some(datos)), anio)
        }
        otro => (otro, None),
    }
}

fn buscar_en_deezer(agente: &ureq::Agent, tema: &TemaBuscado) -> Result<Option<DatosDescriptivos>, String> {
    let terminos = format!("{} {}", tema.artista.as_deref().unwrap_or_default(), sin_parentesis(&tema.titulo));
    let respuesta = pedir_json(agente, DEEZER_TEMAS, &[("q", terminos.as_str()), ("limit", RESULTADOS)])?;
    if let Some(error) = respuesta.get("error") {
        return Err(tr!("Deezer rechazo la busqueda: {error}", error = error));
    }
    let Some((mut datos, album)) = elegir_de_deezer(&respuesta, tema) else {
        return Ok(None);
    };
    // La ficha del album trae el año, el genero y si es un recopilatorio; sin
    // ella no se confia en el album que nombra la busqueda.
    let ficha = album.map(|album| {
        pedir_json(agente, &format!("{DEEZER_ALBUM}/{album}"), &[])
            .inspect_err(|error| log::debug!("Datos de internet: no se pudo leer el album {album} de Deezer: {error}"))
    });
    match ficha {
        Some(Ok(ficha)) => completar_con_album_de_deezer(&mut datos, &ficha),
        _ => datos.album = None,
    }
    Ok(Some(datos))
}

/// El tema que corresponde de una busqueda de Deezer: titulo, artista y
/// album (sin confirmar que no sea un recopilatorio) y el id del album.
fn elegir_de_deezer(respuesta: &Value, tema: &TemaBuscado) -> Option<(DatosDescriptivos, Option<u64>)> {
    let candidatos = respuesta.get("data")?.as_array()?.iter().filter_map(|resultado| {
        let titulo = resultado.get("title_short").or_else(|| resultado.get("title"))?.as_str()?;
        let artista = resultado.get("artist")?.get("name")?.as_str()?;
        let duracion = resultado.get("duration").and_then(Value::as_f64);
        let album = resultado.get("album");
        Some(Candidato {
            titulo: titulo.to_string(),
            artista: artista.to_string(),
            duracion,
            datos: DatosDescriptivos {
                titulo: Some(titulo.to_string()),
                artista: Some(artista.to_string()),
                album: album.and_then(|album| album.get("title")).and_then(Value::as_str).map(str::to_string),
                ..DatosDescriptivos::default()
            },
            id_de_album: album.and_then(|album| album.get("id")).and_then(Value::as_u64),
        })
    });
    elegir(candidatos, tema).map(|candidato| (candidato.datos, candidato.id_de_album))
}

/// Con la ficha del album de Deezer: el año y el genero; y el album solo si
/// es un album (no un recopilatorio ni un simple).
fn completar_con_album_de_deezer(datos: &mut DatosDescriptivos, ficha: &Value) {
    let tipo = ficha.get("record_type").and_then(Value::as_str).unwrap_or_default();
    let nombre = ficha.get("title").and_then(Value::as_str).or(datos.album.as_deref()).unwrap_or_default().to_string();
    let de_varios = ficha
        .get("artist")
        .and_then(|artista| artista.get("name"))
        .and_then(Value::as_str)
        .is_some_and(|nombre| comparable(nombre) == "various artists");
    let recopilatorio = tipo == "compile" || de_varios || parece_recopilatorio(&nombre);
    datos.album = (tipo == "album" && !recopilatorio).then(|| limpiar_album(&nombre));
    if !recopilatorio {
        datos.anio = ficha.get("release_date").and_then(Value::as_str).and_then(anio_de);
    }
    datos.genero = ficha
        .get("genres")
        .and_then(|generos| generos.get("data"))
        .and_then(Value::as_array)
        .and_then(|generos| generos.first())
        .and_then(|genero| genero.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
}

fn buscar_en_itunes(agente: &ureq::Agent, tema: &TemaBuscado) -> Result<Option<DatosDescriptivos>, String> {
    let terminos = format!("{} {}", tema.artista.as_deref().unwrap_or_default(), sin_parentesis(&tema.titulo));
    let respuesta = pedir_json(agente, ITUNES, &[("term", terminos.as_str()), ("entity", "song"), ("limit", RESULTADOS)])?;
    Ok(elegir_de_itunes(&respuesta, tema))
}

/// El tema que corresponde de una busqueda de iTunes. El año es el mas
/// antiguo entre todas las versiones del tema que coinciden (las demas son
/// reediciones o remasterizaciones).
fn elegir_de_itunes(respuesta: &Value, tema: &TemaBuscado) -> Option<DatosDescriptivos> {
    let candidatos: Vec<Candidato> = respuesta.get("results")?.as_array()?.iter().filter_map(|resultado| {
        let texto = |clave: &str| resultado.get(clave).and_then(Value::as_str).map(str::to_string);
        let titulo = texto("trackName")?;
        let artista = texto("artistName")?;
        let album = texto("collectionName");
        // Un album de varios artistas o con nombre de recopilacion: no se
        // toman ni el album ni el año.
        let recopilatorio = texto("collectionArtistName").is_some_and(|nombre| comparable(&nombre) == "various artists")
            || album.as_deref().is_some_and(parece_recopilatorio);
        Some(Candidato {
            titulo: titulo.clone(),
            artista: artista.clone(),
            duracion: resultado.get("trackTimeMillis").and_then(Value::as_f64).map(|milisegundos| milisegundos / 1000.0),
            datos: DatosDescriptivos {
                titulo: Some(titulo),
                artista: Some(artista),
                album: album.filter(|_| !recopilatorio).map(|album| limpiar_album(&album)),
                anio: texto("releaseDate").and_then(|fecha| anio_de(&fecha)).filter(|_| !recopilatorio),
                genero: texto("primaryGenreName"),
            },
            id_de_album: None,
        })
    }).collect();
    let mas_antiguo = candidatos.iter().filter(|candidato| coincidencia(candidato, tema).is_some()).filter_map(|candidato| candidato.datos.anio).min();
    let mut elegido = elegir(candidatos.into_iter(), tema)?.datos;
    elegido.anio = mas_antiguo;
    Some(elegido)
}

/// Un resultado de Deezer o iTunes.
struct Candidato {
    titulo: String,
    artista: String,
    duracion: Option<f64>,
    datos: DatosDescriptivos,
    id_de_album: Option<u64>,
}

/// El candidato del mismo artista, con duracion parecida (si se conocen las
/// dos) y el mismo titulo; si ninguno tiene el mismo, el primero que solo le
/// agrega palabras ("Tema - Remastered 2011").
fn elegir(candidatos: impl Iterator<Item = Candidato>, tema: &TemaBuscado) -> Option<Candidato> {
    let mut parecido = None;
    for candidato in candidatos {
        match coincidencia(&candidato, tema) {
            Some(true) => return Some(candidato),
            Some(false) if parecido.is_none() => parecido = Some(candidato),
            _ => {}
        }
    }
    parecido
}

/// Si el candidato es el tema buscado: Some(true) con el mismo titulo,
/// Some(false) si solo le agrega palabras, None si no es (otro artista,
/// otro titulo u otra duracion).
fn coincidencia(candidato: &Candidato, tema: &TemaBuscado) -> Option<bool> {
    let artista = tema.artista.as_deref()?;
    let misma_duracion = match (candidato.duracion, tema.duracion) {
        (Some(una), Some(otra)) => (una - otra).abs() <= TOLERANCIA_DE_DURACION,
        _ => true,
    };
    if !misma_duracion || !mismo_artista(&candidato.artista, artista) {
        return None;
    }
    let (titulo, buscado) = (comparable(&sin_parentesis(&candidato.titulo)), comparable(&sin_parentesis(&tema.titulo)));
    if titulo == buscado {
        Some(true)
    } else {
        agrega_palabras(&titulo, &buscado).then_some(false)
    }
}

/// Si el nombre de un album parece el de un recopilatorio.
fn parece_recopilatorio(nombre: &str) -> bool {
    let nombre = format!(" {} ", comparable(nombre));
    DE_RECOPILATORIO.iter().any(|palabras| nombre.contains(&format!(" {palabras} ")))
}

/// El nombre del album sin la aclaracion de la edicion: "A Night at the
/// Opera (Deluxe Edition)" o "Rumours - 2004 Remaster" dan el album.
fn limpiar_album(nombre: &str) -> String {
    let de_edicion = |texto: &str| {
        let texto = comparable(texto);
        DE_EDICION.iter().any(|palabra| texto.split(' ').any(|otra| otra == *palabra))
    };
    let mut limpio = String::with_capacity(nombre.len());
    let mut resto = nombre;
    while let Some(inicio) = resto.find(['(', '[']) {
        let cierre = if resto[inicio..].starts_with('(') { ')' } else { ']' };
        let Some(fin) = resto[inicio..].find(cierre).map(|fin| inicio + fin) else { break };
        limpio.push_str(&resto[..inicio]);
        if !de_edicion(&resto[inicio + 1..fin]) {
            limpio.push_str(&resto[inicio..=fin]);
        }
        resto = &resto[fin + 1..];
    }
    limpio.push_str(resto);
    // " - 2004 Remaster" al final.
    if let Some((antes, despues)) = limpio.rsplit_once(" - ")
        && de_edicion(despues)
    {
        limpio = antes.to_string();
    }
    let limpio = limpio.split_whitespace().collect::<Vec<_>>().join(" ");
    if limpio.is_empty() { nombre.trim().to_string() } else { limpio }
}

/// El año de una fecha "1997-09-09" o "1997-09-09T07:00:00Z".
fn anio_de(fecha: &str) -> Option<u16> {
    fecha.get(..4)?.parse().ok().filter(|anio| crate::audio::anio_valido(*anio))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn tema(titulo: &str, artista: &str, duracion: Option<f64>) -> TemaBuscado {
        TemaBuscado { titulo: titulo.into(), artista: Some(artista.into()), duracion }
    }

    #[test]
    fn cada_fuente_completa_solo_lo_que_falta() {
        let mut encontrado = DatosDeInternet::default();
        let musicbrainz = DatosDescriptivos { titulo: Some("Amiga mía".into()), album: Some("Más".into()), ..DatosDescriptivos::default() };
        encontrado.sumar(musicbrainz, FuenteDeDatos::MusicBrainz);
        let deezer = DatosDescriptivos {
            titulo: Some("Amiga Mia".into()),
            album: Some("Otro".into()),
            anio: Some(1997),
            genero: Some("  ".into()),
            ..DatosDescriptivos::default()
        };
        encontrado.sumar(deezer, FuenteDeDatos::Deezer);
        let itunes = DatosDescriptivos { genero: Some("Pop latino".into()), anio: Some(2004), ..DatosDescriptivos::default() };
        encontrado.sumar(itunes, FuenteDeDatos::Itunes);
        assert_eq!(encontrado.datos.titulo.as_deref(), Some("Amiga mía"));
        assert_eq!(encontrado.datos.album.as_deref(), Some("Más"));
        assert_eq!(encontrado.datos.anio, Some(1997));
        assert_eq!(encontrado.datos.genero.as_deref(), Some("Pop latino"), "un genero vacio no cuenta");
        assert_eq!(encontrado.fuente_de(CampoDescriptivo::Album), Some(FuenteDeDatos::MusicBrainz));
        assert_eq!(encontrado.fuente_de(CampoDescriptivo::Anio), Some(FuenteDeDatos::Deezer));
        assert_eq!(encontrado.fuente_de(CampoDescriptivo::Genero), Some(FuenteDeDatos::Itunes));
        assert_eq!(encontrado.fuente_de(CampoDescriptivo::Artista), None);
        assert!(!encontrado.completo());
    }

    #[test]
    fn de_deezer_elige_el_tema_y_descarta_el_album_recopilatorio() {
        let busqueda: Value = serde_json::from_str(
            r#"{"data": [
                {"title": "All Out of Love", "artist": {"name": "Otro"}, "duration": 241, "album": {"id": 1, "title": "X"}},
                {"title": "All Out of Love (Live)", "title_short": "All Out of Love", "artist": {"name": "Air Supply"}, "duration": 330, "album": {"id": 2, "title": "En vivo"}},
                {"title": "All Out of Love", "artist": {"name": "Air Supply"}, "duration": 240, "album": {"id": 3, "title": "Lost in Love"}}
            ]}"#,
        )
        .unwrap();
        let (datos, album) = elegir_de_deezer(&busqueda, &tema("All out of love", "AIR SUPPLY", Some(241.0))).unwrap();
        assert_eq!((datos.album.as_deref(), album), (Some("Lost in Love"), Some(3)), "descarta el de otra duracion");
        let mut del_album = datos.clone();
        let ficha: Value = serde_json::from_str(r#"{"title": "Lost in Love", "record_type": "album", "release_date": "1980-03-01", "genres": {"data": [{"name": "Pop"}]}}"#).unwrap();
        completar_con_album_de_deezer(&mut del_album, &ficha);
        assert_eq!((del_album.album.as_deref(), del_album.anio, del_album.genero.as_deref()), (Some("Lost in Love"), Some(1980), Some("Pop")));
        let mut recopilado = datos;
        let ficha: Value = serde_json::from_str(r#"{"title": "Ultimate Air Supply", "record_type": "compile", "release_date": "2005-01-01", "genres": {"data": [{"name": "Pop"}]}}"#).unwrap();
        completar_con_album_de_deezer(&mut recopilado, &ficha);
        assert_eq!((recopilado.album, recopilado.anio, recopilado.genero.as_deref()), (None, None, Some("Pop")));
    }

    #[test]
    fn de_itunes_toma_el_genero_aunque_sea_un_recopilatorio() {
        let respuesta: Value = serde_json::from_str(
            r#"{"results": [
                {"trackName": "Bohemian Rhapsody", "artistName": "Queen", "collectionName": "Greatest Hits", "releaseDate": "1981-10-26T08:00:00Z", "primaryGenreName": "Rock", "trackTimeMillis": 355000},
                {"trackName": "Bohemian Rhapsody - Remastered 2011", "artistName": "Queen", "collectionName": "A Night at the Opera", "releaseDate": "1975-11-21T08:00:00Z", "primaryGenreName": "Rock", "trackTimeMillis": 355000}
            ]}"#,
        )
        .unwrap();
        // El titulo exacto gana aunque su album sea un recopilatorio: de el
        // quedan el titulo y el genero, no el album; el año es el mas antiguo
        // de las versiones que coinciden (la remasterizada es de 1975).
        let datos = elegir_de_itunes(&respuesta, &tema("Bohemian Rhapsody", "Queen", Some(354.0))).unwrap();
        assert_eq!((datos.album, datos.anio, datos.genero.as_deref()), (None, Some(1975), Some("Rock")));
        // Con otra duracion no hay coincidencia.
        assert!(elegir_de_itunes(&respuesta, &tema("Bohemian Rhapsody", "Queen", Some(200.0))).is_none());
        assert!(elegir_de_itunes(&respuesta, &tema("Bohemian Rhapsody", "Queensryche", None)).is_none());
    }

    #[test]
    fn el_tema_saneado_quita_el_numero_de_pista() {
        let saneado = TemaBuscado { titulo: "03. Lullaby".into(), artista: Some("05. THE CURE".into()), duracion: Some(250.0) }.saneado().unwrap();
        assert_eq!((saneado.titulo.as_str(), saneado.artista.as_deref(), saneado.duracion), ("Lullaby", Some("THE CURE"), Some(250.0)));
        assert!(tema("Lullaby", "The Cure", None).saneado().is_none());
    }

    #[test]
    fn reconoce_los_recopilatorios_y_lee_el_anio() {
        for nombre in ["Greatest Hits", "The Best of Queen", "Grandes Éxitos", "Lo mejor de Sandra", "Gold - Greatest Hits", "Collections", "Ultimate Air Supply"] {
            assert!(parece_recopilatorio(nombre), "{nombre}");
        }
        assert_eq!(limpiar_album("A Night At The Opera (Deluxe Edition)"), "A Night At The Opera");
        assert_eq!(limpiar_album("Rumours - 2004 Remaster"), "Rumours");
        assert_eq!(limpiar_album("Chicago 16 (Expanded & Remastered)"), "Chicago 16");
        assert_eq!(limpiar_album("Comfort y música para volar (En vivo)"), "Comfort y música para volar (En vivo)", "en vivo es otro album");
        assert_eq!(limpiar_album("(Deluxe)"), "(Deluxe)", "no queda vacio");
        for nombre in ["Lost in Love", "Hitsville", "Más", "A Night at the Opera"] {
            assert!(!parece_recopilatorio(nombre), "{nombre}");
        }
        assert_eq!(anio_de("1997-09-09"), Some(1997));
        assert_eq!(anio_de("1975-11-21T08:00:00Z"), Some(1975));
        assert_eq!(anio_de("0000-00-00"), None);
        assert_eq!(anio_de("x"), None);
    }

    /// Busqueda real (necesita internet): se corre a mano con
    /// `cargo test datos_reales -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn datos_reales_de_internet() {
        let encontrado = buscar_datos(&tema("All Out of Love", "Air Supply", Some(241.0))).unwrap().unwrap();
        println!("{encontrado:?}");
        assert!(encontrado.datos.album.is_some() && encontrado.datos.anio.is_some());
        let agente: ureq::Agent = ureq::Agent::config_builder().user_agent(IDENTIFICACION).build().into();
        let deezer = buscar_en_deezer(&agente, &tema("Bohemian Rhapsody", "Queen", None)).unwrap();
        let itunes = buscar_en_itunes(&agente, &tema("Bohemian Rhapsody", "Queen", None)).unwrap();
        println!("Deezer: {deezer:?}\niTunes: {itunes:?}");
        assert!(deezer.is_some() && itunes.is_some());
        // Con el numero de pista pegado al artista, aparece al sanearlo.
        let saneado = buscar_datos(&tema("03. Lullaby", "05. THE CURE", None)).unwrap();
        println!("Saneado: {saneado:?}");
        assert!(saneado.is_some());
    }
}
