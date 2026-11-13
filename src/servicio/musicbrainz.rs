//! Consulta a MusicBrainz (https://musicbrainz.org), la base libre de datos
//! musicales que usa Picard. Para cada tema se hacen dos consultas:
//!
//! 1. Una busqueda por titulo y artista, de la que se elige la grabacion
//!    original: el mismo titulo, duracion parecida (si se conoce) y la
//!    primera edicion mas antigua (no una reedicion ni un compilado).
//! 2. La ficha de esa grabacion, que trae sus generos y sus ediciones, para
//!    elegir el album de estudio donde salio.
//!
//! No necesita clave; pide un User-Agent con un contacto y como mucho una
//! consulta por segundo.

use std::time::Duration;

use serde_json::Value;

use crate::dominio::{DatosDescriptivos, simplificar_nombre};

/// Pausa entre consultas: MusicBrainz admite una por segundo.
pub const PAUSA_ENTRE_CONSULTAS: Duration = Duration::from_millis(1_100);
const TIEMPO_MAXIMO_DE_LA_CONSULTA: Duration = Duration::from_secs(15);
const DIRECCION: &str = "https://musicbrainz.org/ws/2/recording";
/// Identificacion que pide MusicBrainz: programa, version y un contacto (el
/// publico del proyecto).
const IDENTIFICACION: &str = concat!("MCPRaptorPlayer/", env!("CARGO_PKG_VERSION"), " ( jxlfybcrk@mozmail.com )");
const CANDIDATOS_POR_CONSULTA: u8 = 15;
/// Puntaje minimo (0 a 100) de MusicBrainz para aceptar una coincidencia.
const PUNTAJE_MINIMO: u64 = 90;
/// Diferencia maxima de duracion para aceptar la grabacion, si se conoce.
const TOLERANCIA_DE_DURACION: f64 = 8.0;

/// Grabacion de MusicBrainz que corresponde a un tema.
#[derive(Clone, Debug, PartialEq)]
pub struct Coincidencia {
    pub datos: DatosDescriptivos,
    /// Identificador de la grabacion (para ver la pagina en MusicBrainz).
    pub id: String,
}

/// Busca un tema por titulo y, si se conoce, artista; con la duracion se
/// descartan las grabaciones de otra version (en vivo, extendida). None si
/// no hay una coincidencia segura. Bloquea hasta que responde (dos consultas,
/// con la pausa entre ellas).
pub fn buscar_grabacion(titulo: &str, artista: Option<&str>, duracion: Option<f64>) -> Result<Option<Coincidencia>, String> {
    let agente: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIEMPO_MAXIMO_DE_LA_CONSULTA))
        .user_agent(IDENTIFICACION)
        .build()
        .into();
    let consulta = armar_consulta(titulo, artista);
    let busqueda = pedir(&agente, DIRECCION, &[("query", consulta.as_str()), ("limit", &CANDIDATOS_POR_CONSULTA.to_string())])?;
    let Some(id) = elegir_grabacion(&busqueda, titulo, duracion)? else {
        return Ok(None);
    };
    std::thread::sleep(PAUSA_ENTRE_CONSULTAS);
    let ficha = pedir(&agente, &format!("{DIRECCION}/{id}"), &[("inc", "artist-credits+releases+release-groups+genres+tags")])?;
    Ok(Some(Coincidencia { datos: interpretar_grabacion(&ficha)?, id }))
}

/// Una consulta GET en JSON. Si el servicio esta ocupado (503), se reintenta
/// una vez despues de una pausa.
fn pedir(agente: &ureq::Agent, direccion: &str, parametros: &[(&str, &str)]) -> Result<String, String> {
    let consultar = || {
        let mut pedido = agente.get(direccion).query("fmt", "json");
        for (clave, valor) in parametros {
            pedido = pedido.query(*clave, *valor);
        }
        pedido.call()
    };
    let respuesta = match consultar() {
        Err(ureq::Error::StatusCode(503)) => {
            std::thread::sleep(PAUSA_ENTRE_CONSULTAS * 2);
            consultar()
        }
        otra => otra,
    };
    respuesta
        .map_err(|error| tr!("No se pudo consultar MusicBrainz: {error}", error = error))?
        .body_mut()
        .read_to_string()
        .map_err(|error| tr!("No se pudo leer la respuesta de MusicBrainz: {error}", error = error))
}

/// Consulta de busqueda de MusicBrainz (sintaxis de Lucene), con las frases
/// entre comillas.
fn armar_consulta(titulo: &str, artista: Option<&str>) -> String {
    let frase = |texto: &str| format!("\"{}\"", texto.trim().replace('\\', "\\\\").replace('"', "\\\""));
    match artista.map(str::trim).filter(|artista| !artista.is_empty()) {
        Some(artista) => format!("recording:{} AND artist:{}", frase(titulo), frase(artista)),
        None => format!("recording:{}", frase(titulo)),
    }
}

fn leer_json(texto: &str) -> Result<Value, String> {
    serde_json::from_str(texto).map_err(|error| tr!("Respuesta de MusicBrainz ilegible: {error}", error = error))
}

fn texto_de(valor: Option<&Value>) -> Option<String> {
    valor.and_then(Value::as_str).map(str::trim).filter(|texto| !texto.is_empty()).map(str::to_string)
}

/// Elige, de los resultados de la busqueda, la grabacion original: puntaje
/// suficiente, duracion parecida (si se conocen las dos), el mismo titulo si
/// alguna lo tiene, y de esas la de primera edicion mas antigua.
fn elegir_grabacion(texto: &str, titulo: &str, duracion: Option<f64>) -> Result<Option<String>, String> {
    let raiz = leer_json(texto)?;
    let Some(grabaciones) = raiz.get("recordings").and_then(Value::as_array) else {
        return Ok(None);
    };
    let aceptables: Vec<&Value> = grabaciones
        .iter()
        .filter(|grabacion| grabacion.get("score").and_then(Value::as_u64).unwrap_or(0) >= PUNTAJE_MINIMO)
        .filter(|grabacion| {
            let largo = grabacion.get("length").and_then(Value::as_f64).map(|milisegundos| milisegundos / 1000.0);
            match (largo, duracion) {
                (Some(largo), Some(duracion)) => (largo - duracion).abs() <= TOLERANCIA_DE_DURACION,
                _ => true,
            }
        })
        .collect();
    let buscado = simplificar_nombre(titulo);
    let mismo_titulo: Vec<&Value> = aceptables
        .iter()
        .copied()
        .filter(|grabacion| texto_de(grabacion.get("title")).is_some_and(|nombre| simplificar_nombre(&nombre) == buscado))
        .collect();
    let candidatas = if mismo_titulo.is_empty() { aceptables } else { mismo_titulo };
    // Sin fecha va al final; a igual fecha, la primera (de mayor puntaje).
    let fecha = |grabacion: &&Value| texto_de(grabacion.get("first-release-date")).unwrap_or_else(|| "9999".into());
    Ok(candidatas.into_iter().min_by_key(fecha).and_then(|grabacion| texto_de(grabacion.get("id"))))
}

/// Datos de la ficha de una grabacion (consulta con artist-credits,
/// releases, release-groups, genres y tags).
fn interpretar_grabacion(texto: &str) -> Result<DatosDescriptivos, String> {
    let grabacion = leer_json(texto)?;
    Ok(DatosDescriptivos {
        titulo: texto_de(grabacion.get("title")),
        artista: artista_acreditado(&grabacion),
        album: album_de(&grabacion),
        anio: texto_de(grabacion.get("first-release-date")).and_then(|fecha| fecha.get(..4)?.parse().ok()),
        genero: genero_de(&grabacion, "genres").or_else(|| genero_de(&grabacion, "tags")),
    })
}

/// Artistas acreditados, unidos como figuran ("Charly Garcia & Pedro Aznar").
fn artista_acreditado(grabacion: &Value) -> Option<String> {
    let creditos = grabacion.get("artist-credit")?.as_array()?;
    let nombre: String = creditos
        .iter()
        .map(|credito| {
            let parte = |clave: &str| credito.get(clave).and_then(Value::as_str).unwrap_or_default();
            format!("{}{}", parte("name"), parte("joinphrase"))
        })
        .collect();
    Some(nombre.trim().to_string()).filter(|nombre| !nombre.is_empty())
}

/// El album donde salio: de las ediciones oficiales, el album de estudio (no
/// compilado ni en vivo) mas antiguo; si no hay, la oficial mas antigua.
fn album_de(grabacion: &Value) -> Option<String> {
    let ediciones = grabacion.get("releases")?.as_array()?;
    let oficial = |edicion: &&Value| edicion.get("status").and_then(Value::as_str).is_none_or(|estado| estado == "Official");
    let de_estudio = |edicion: &&Value| {
        let grupo = edicion.get("release-group");
        let tipo = grupo.and_then(|grupo| grupo.get("primary-type")).and_then(Value::as_str);
        let secundarios = grupo.and_then(|grupo| grupo.get("secondary-types")).and_then(Value::as_array).map_or(0, Vec::len);
        tipo == Some("Album") && secundarios == 0
    };
    let fecha = |edicion: &&Value| texto_de(edicion.get("date")).unwrap_or_else(|| "9999".into());
    let elegida = ediciones
        .iter()
        .filter(oficial)
        .filter(de_estudio)
        .min_by_key(fecha)
        .or_else(|| ediciones.iter().filter(oficial).min_by_key(fecha))?;
    texto_de(elegida.get("title"))
}

/// El genero (o la etiqueta) mas votado, con mayuscula inicial ("rock en
/// español" da "Rock en español").
fn genero_de(grabacion: &Value, clave: &str) -> Option<String> {
    let etiquetas = grabacion.get(clave)?.as_array()?;
    let nombre = etiquetas
        .iter()
        .filter_map(|etiqueta| Some((etiqueta.get("count").and_then(Value::as_i64).unwrap_or(0), etiqueta.get("name")?.as_str()?.trim())))
        .filter(|(votos, nombre)| *votos > 0 && !nombre.is_empty())
        .max_by_key(|(votos, _)| *votos)?
        .1;
    let mut letras = nombre.chars();
    let primera = letras.next()?;
    Some(primera.to_uppercase().chain(letras).collect())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const BUSQUEDA: &str = r#"{"recordings": [
        {"id": "reedicion", "score": 100, "title": "Amiga mía", "length": 262000, "first-release-date": "2017-01-01"},
        {"id": "en-vivo", "score": 100, "title": "Amiga mía", "length": 330000, "first-release-date": "1995-01-01"},
        {"id": "otra-cancion", "score": 95, "title": "Amiga", "length": 262000, "first-release-date": "1990-01-01"},
        {"id": "original", "score": 98, "title": "Amiga Mia", "length": 261000, "first-release-date": "1997-09-09"},
        {"id": "dudosa", "score": 60, "title": "Amiga mía", "first-release-date": "1980"}
    ]}"#;

    const FICHA: &str = r#"{"id": "original", "title": "Amiga mía", "first-release-date": "1997-09-09",
        "artist-credit": [{"name": "Alejandro Sanz", "joinphrase": " & "}, {"name": "Otro"}],
        "genres": [{"count": 1, "name": "pop"}, {"count": 3, "name": "pop latino"}],
        "tags": [{"count": 9, "name": "favoritos"}],
        "releases": [
            {"title": "Grandes éxitos", "status": "Official", "date": "2004", "release-group": {"primary-type": "Album", "secondary-types": ["Compilation"]}},
            {"title": "Más", "status": "Official", "date": "1997-09-09", "release-group": {"primary-type": "Album", "secondary-types": []}},
            {"title": "Más (edición especial)", "status": "Official", "date": "1998", "release-group": {"primary-type": "Album"}}
        ]}"#;

    #[test]
    fn elige_la_grabacion_original_del_mismo_titulo() {
        // Con la duracion: descarta la version en vivo y elige la mas antigua.
        assert_eq!(elegir_grabacion(BUSQUEDA, "Amiga mía", Some(262.0)).unwrap().as_deref(), Some("original"));
        // Sin la duracion vale la version en vivo de 1995, la mas antigua.
        assert_eq!(elegir_grabacion(BUSQUEDA, "amiga mia", None).unwrap().as_deref(), Some("en-vivo"));
        let dudosa = r#"{"recordings": [{"id": "x", "score": 70, "title": "Otra"}]}"#;
        assert_eq!(elegir_grabacion(dudosa, "Otra", None).unwrap(), None);
        assert_eq!(elegir_grabacion(r#"{"recordings": []}"#, "Otra", None).unwrap(), None);
        assert!(elegir_grabacion("no es json", "Otra", None).is_err());
    }

    #[test]
    fn de_la_ficha_toma_el_album_de_estudio_y_el_genero_mas_votado() {
        let datos = interpretar_grabacion(FICHA).unwrap();
        assert_eq!(datos.titulo.as_deref(), Some("Amiga mía"));
        assert_eq!(datos.artista.as_deref(), Some("Alejandro Sanz & Otro"));
        assert_eq!(datos.album.as_deref(), Some("Más"));
        assert_eq!(datos.anio, Some(1997));
        assert_eq!(datos.genero.as_deref(), Some("Pop latino"), "los generos van antes que las etiquetas");
        let sin_generos = FICHA.replace(r#""genres""#, r#""otros""#);
        assert_eq!(interpretar_grabacion(&sin_generos).unwrap().genero.as_deref(), Some("Favoritos"));
    }

    #[test]
    fn la_consulta_escapa_las_comillas() {
        assert_eq!(armar_consulta("Te \"quiero\"", Some(" Sanz ")), r#"recording:"Te \"quiero\"" AND artist:"Sanz""#);
        assert_eq!(armar_consulta("Tema", None), r#"recording:"Tema""#);
    }

    /// Consulta real (necesita internet): se corre a mano con
    /// `cargo test musicbrainz -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn consulta_real_a_musicbrainz() {
        for (titulo, artista) in [("Amiga mía", "Alejandro Sanz"), ("De música ligera", "Soda Stereo")] {
            let coincidencia = buscar_grabacion(titulo, Some(artista), None).unwrap();
            println!("{titulo}: {coincidencia:?}");
            assert!(coincidencia.is_some(), "{titulo}");
            std::thread::sleep(PAUSA_ENTRE_CONSULTAS);
        }
    }
}
