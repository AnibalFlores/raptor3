//! Consulta de temas con prefijos, para el buscador de la lista y el del
//! catalogo.
//!
//! Sin prefijos se comporta como siempre: el tema coincide si tiene todas las
//! palabras en su titulo, artista, genero, album, tipo o archivo, sin
//! importar mayusculas ni acentos. Ademas entiende:
//!
//! - `#508`: la posicion 508 de la lista (`#10..20`, un tramo).
//! - `campo:valor` para buscar solo en un dato: `artista:sanz`,
//!   `genero:"rock nacional"` (con comillas, una frase).
//! - Comparaciones en los datos con numero: `ranking:>=4`, `anio:1990..1999`,
//!   `bpm:120-128`, `duracion:<3:30`.
//! - La ultima reproduccion: `sonado:hoy`, `sonado:7d` (en los ultimos 7
//!   dias), `sonado:>30d` (hace mas de 30 dias), `sonado:2026-11-03`,
//!   `sonado:<01/10/2026`, `sonado:nunca`.
//! - `-` adelante niega cualquier condicion: `-genero:cumbia`.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, Zoned};

use super::duplicados::sin_acento;

/// Datos de un tema que mira la consulta. Lo que no se sabe va vacio o None.
#[derive(Clone, Copy, Debug, Default)]
pub struct DatosDelTema<'a> {
    pub titulo: &'a str,
    pub artista: &'a str,
    pub genero: &'a str,
    pub album: &'a str,
    pub tipo: &'a str,
    pub nota: &'a str,
    /// Lo que se mira del archivo en la busqueda sin prefijo: la ruta en la
    /// lista, solo el nombre en el catalogo.
    pub archivo: &'a str,
    /// Ruta completa, para `archivo:` y `ruta:`.
    pub ruta: &'a str,
    pub anio: Option<u16>,
    pub ranking: Option<u8>,
    pub bpm: Option<f64>,
    /// Segundos.
    pub duracion: Option<f64>,
    /// Fecha y hora UTC en ISO 8601, como la guarda el catalogo.
    pub ultima_reproduccion: Option<&'a str>,
    /// Posicion en la lista, contando desde 1 (None fuera de la lista).
    pub posicion: Option<usize>,
}

/// Dato de texto en que busca un prefijo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CampoDeTexto {
    Titulo,
    Artista,
    Genero,
    Album,
    Tipo,
    Nota,
    Archivo,
}

/// Dato con numero que se compara.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CampoNumerico {
    Posicion,
    Anio,
    Ranking,
    Bpm,
    Duracion,
}

/// Valores admitidos, como intervalo semiabierto `[desde, hasta)`; sin
/// limite, abierto de ese lado.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Intervalo {
    desde: Option<f64>,
    hasta: Option<f64>,
}

impl Intervalo {
    fn contiene(&self, valor: f64) -> bool {
        self.desde.is_none_or(|desde| valor >= desde) && self.hasta.is_none_or(|hasta| valor < hasta)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Criterio {
    /// En todos los datos de texto generales.
    Texto(String),
    Campo(CampoDeTexto, String),
    Numero(CampoNumerico, Intervalo),
    /// Ultima reproduccion dentro del intervalo (segundos desde 1970).
    Sonado(Intervalo),
    /// Nunca se reprodujo.
    NuncaSonado,
}

#[derive(Clone, Debug, PartialEq)]
struct Condicion {
    negada: bool,
    criterio: Criterio,
}

/// Consulta armada con el texto del buscador: el tema coincide si cumple
/// todas las condiciones.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsultaDeTemas {
    condiciones: Vec<Condicion>,
}

impl ConsultaDeTemas {
    /// Ok(None) si el texto esta vacio (sin busqueda). Err con la
    /// explicacion si un prefijo tiene un valor que no se entiende.
    pub fn nueva(texto: &str) -> Result<Option<Self>, String> {
        Self::nueva_en(texto, &Zoned::now())
    }

    /// Como `nueva`, con el momento y la zona horaria con que se resuelven
    /// las fechas relativas ("hoy", "7d").
    pub fn nueva_en(texto: &str, ahora: &Zoned) -> Result<Option<Self>, String> {
        let mut condiciones = Vec::new();
        for termino in separar_terminos(texto) {
            let (negada, termino) = match termino.strip_prefix('-') {
                Some(resto) if !resto.is_empty() => (true, resto.to_string()),
                _ => (false, termino),
            };
            condiciones.push(Condicion { negada, criterio: interpretar_termino(&termino, ahora)? });
        }
        Ok((!condiciones.is_empty()).then_some(Self { condiciones }))
    }

    /// Si busca por posicion en la lista (`#508`), que fuera de ella no tiene sentido.
    pub fn usa_la_posicion(&self) -> bool {
        self.condiciones.iter().any(|condicion| matches!(condicion.criterio, Criterio::Numero(CampoNumerico::Posicion, _)))
    }

    pub fn coincide(&self, datos: &DatosDelTema) -> bool {
        self.condiciones.iter().all(|condicion| condicion.criterio.cumple(datos) != condicion.negada)
    }
}

impl Criterio {
    fn cumple(&self, datos: &DatosDelTema) -> bool {
        match self {
            Criterio::Texto(buscado) => {
                let generales = [datos.titulo, datos.artista, datos.genero, datos.album, datos.tipo, datos.archivo];
                generales.iter().any(|dato| simplificar(dato).contains(buscado.as_str()))
            }
            Criterio::Campo(campo, buscado) => simplificar(campo.de(datos)).contains(buscado.as_str()),
            Criterio::Numero(campo, intervalo) => campo.de(datos).is_some_and(|valor| intervalo.contiene(valor)),
            Criterio::Sonado(intervalo) => instante_de(datos).is_some_and(|segundos| intervalo.contiene(segundos)),
            Criterio::NuncaSonado => instante_de(datos).is_none(),
        }
    }
}

impl CampoDeTexto {
    fn de<'a>(self, datos: &DatosDelTema<'a>) -> &'a str {
        match self {
            CampoDeTexto::Titulo => datos.titulo,
            CampoDeTexto::Artista => datos.artista,
            CampoDeTexto::Genero => datos.genero,
            CampoDeTexto::Album => datos.album,
            CampoDeTexto::Tipo => datos.tipo,
            CampoDeTexto::Nota => datos.nota,
            CampoDeTexto::Archivo => datos.ruta,
        }
    }
}

impl CampoNumerico {
    fn de(self, datos: &DatosDelTema) -> Option<f64> {
        match self {
            CampoNumerico::Posicion => datos.posicion.map(|posicion| posicion as f64),
            CampoNumerico::Anio => datos.anio.map(f64::from),
            CampoNumerico::Ranking => datos.ranking.map(f64::from),
            CampoNumerico::Bpm => datos.bpm,
            CampoNumerico::Duracion => datos.duracion,
        }
    }

    fn nombre(self) -> String {
        match self {
            CampoNumerico::Posicion => "#".into(),
            CampoNumerico::Anio => tr!("anio"),
            CampoNumerico::Ranking => tr!("ranking"),
            CampoNumerico::Bpm => "bpm".into(),
            CampoNumerico::Duracion => tr!("duracion"),
        }
    }
}

/// Segundos desde 1970 de la ultima reproduccion, si se sabe.
fn instante_de(datos: &DatosDelTema) -> Option<f64> {
    let instante = datos.ultima_reproduccion?.parse::<Timestamp>().ok()?;
    Some(instante.as_second() as f64)
}

/// Lo que se puede buscar con un prefijo.
enum Prefijo {
    Texto(CampoDeTexto),
    Numero(CampoNumerico),
    Sonado,
}

/// Prefijos admitidos (ya simplificados, sin acentos) con sus abreviaturas,
/// en español y en inglés: valen los dos sin importar el idioma de la interfaz.
fn prefijo(nombre: &str) -> Option<Prefijo> {
    Some(match nombre {
        "titulo" | "t" | "title" => Prefijo::Texto(CampoDeTexto::Titulo),
        "artista" | "a" | "artist" => Prefijo::Texto(CampoDeTexto::Artista),
        "genero" | "g" | "genre" => Prefijo::Texto(CampoDeTexto::Genero),
        "album" => Prefijo::Texto(CampoDeTexto::Album),
        "tipo" | "type" => Prefijo::Texto(CampoDeTexto::Tipo),
        "nota" | "note" => Prefijo::Texto(CampoDeTexto::Nota),
        "archivo" | "ruta" | "file" | "path" => Prefijo::Texto(CampoDeTexto::Archivo),
        "pos" | "posicion" | "position" => Prefijo::Numero(CampoNumerico::Posicion),
        "anio" | "ano" | "year" => Prefijo::Numero(CampoNumerico::Anio),
        "ranking" | "r" | "estrellas" | "rating" | "stars" => Prefijo::Numero(CampoNumerico::Ranking),
        "bpm" => Prefijo::Numero(CampoNumerico::Bpm),
        "duracion" | "dur" | "duration" => Prefijo::Numero(CampoNumerico::Duracion),
        "sonado" | "sono" | "ultima" | "played" | "last" => Prefijo::Sonado,
        _ => return None,
    })
}

fn interpretar_termino(termino: &str, ahora: &Zoned) -> Result<Criterio, String> {
    if let Some(posicion) = termino.strip_prefix('#') {
        if !posicion.is_empty() {
            return interpretar_numero(CampoNumerico::Posicion, posicion).map(|intervalo| Criterio::Numero(CampoNumerico::Posicion, intervalo));
        }
    }
    if let Some((nombre, valor)) = termino.split_once(':') {
        if let Some(prefijo) = prefijo(&simplificar(nombre)) {
            let valor = valor.trim();
            if valor.is_empty() {
                return Err(tr!("Falta el valor después de «{nombre}:»", nombre = nombre));
            }
            return match prefijo {
                Prefijo::Texto(campo) => Ok(Criterio::Campo(campo, simplificar(valor))),
                Prefijo::Numero(campo) => interpretar_numero(campo, valor).map(|intervalo| Criterio::Numero(campo, intervalo)),
                Prefijo::Sonado => interpretar_sonado(valor, ahora),
            };
        }
    }
    // Sin un prefijo conocido ("12:30", "http:"), es una palabra mas.
    Ok(Criterio::Texto(simplificar(termino)))
}

/// Separa el texto en terminos por los espacios, respetando las comillas:
/// `artista:"los piojos" rock` da `artista:los piojos` y `rock`.
fn separar_terminos(texto: &str) -> Vec<String> {
    let mut terminos = Vec::new();
    let mut actual = String::new();
    let mut entre_comillas = false;
    for caracter in texto.chars() {
        match caracter {
            '"' | '«' | '»' | '“' | '”' => entre_comillas = !entre_comillas,
            espacio if espacio.is_whitespace() && !entre_comillas => {
                if !actual.is_empty() {
                    terminos.push(std::mem::take(&mut actual));
                }
            }
            otro => actual.push(otro),
        }
    }
    if !actual.trim().is_empty() {
        terminos.push(actual);
    }
    terminos
}

/// Comparacion escrita: `=v`, `>v`, `>=v`, `<v`, `<=v` o un tramo `a..b`.
enum Comparacion<'a> {
    Igual(&'a str),
    Mayor(&'a str),
    MayorOIgual(&'a str),
    Menor(&'a str),
    MenorOIgual(&'a str),
    Tramo(Option<&'a str>, Option<&'a str>),
}

fn separar_comparacion(valor: &str, guion_es_tramo: bool) -> Comparacion<'_> {
    let valor = valor.trim();
    if let Some(resto) = valor.strip_prefix(">=") {
        return Comparacion::MayorOIgual(resto.trim());
    }
    if let Some(resto) = valor.strip_prefix("<=") {
        return Comparacion::MenorOIgual(resto.trim());
    }
    if let Some(resto) = valor.strip_prefix('>') {
        return Comparacion::Mayor(resto.trim());
    }
    if let Some(resto) = valor.strip_prefix('<') {
        return Comparacion::Menor(resto.trim());
    }
    if let Some(resto) = valor.strip_prefix('=') {
        return Comparacion::Igual(resto.trim());
    }
    let tramo = valor.split_once("..").or_else(|| guion_es_tramo.then(|| valor.split_once('-')).flatten());
    match tramo {
        Some((desde, hasta)) => {
            Comparacion::Tramo(parte_del_tramo(desde), parte_del_tramo(hasta))
        }
        None => Comparacion::Igual(valor),
    }
}

/// Un extremo de un tramo; vacio, sin limite de ese lado.
fn parte_del_tramo(texto: &str) -> Option<&str> {
    Some(texto.trim()).filter(|texto| !texto.is_empty())
}

/// Arma el intervalo de una comparacion. `leer` da, para un valor escrito,
/// el intervalo `[inicio, fin)` que representa (un dia, un minuto, una
/// unidad).
fn intervalo_de(comparacion: Comparacion, leer: impl Fn(&str) -> Option<(f64, f64)>) -> Option<Intervalo> {
    Some(match comparacion {
        Comparacion::Igual(valor) => {
            let (inicio, fin) = leer(valor)?;
            Intervalo { desde: Some(inicio), hasta: Some(fin) }
        }
        Comparacion::Mayor(valor) => Intervalo { desde: Some(leer(valor)?.1), hasta: None },
        Comparacion::MayorOIgual(valor) => Intervalo { desde: Some(leer(valor)?.0), hasta: None },
        Comparacion::Menor(valor) => Intervalo { desde: None, hasta: Some(leer(valor)?.0) },
        Comparacion::MenorOIgual(valor) => Intervalo { desde: None, hasta: Some(leer(valor)?.1) },
        Comparacion::Tramo(None, None) => return None,
        Comparacion::Tramo(desde, hasta) => Intervalo {
            desde: match desde {
                Some(valor) => Some(leer(valor)?.0),
                None => None,
            },
            hasta: match hasta {
                Some(valor) => Some(leer(valor)?.1),
                None => None,
            },
        },
    })
}

fn interpretar_numero(campo: CampoNumerico, valor: &str) -> Result<Intervalo, String> {
    let leer = |texto: &str| -> Option<(f64, f64)> {
        match campo {
            // Minutos ("4", de 4:00 a 4:59) o minutos y segundos ("3:30").
            CampoNumerico::Duracion => match texto.split_once(':') {
                Some((minutos, segundos)) => {
                    let total = minutos.trim().parse::<u32>().ok()? * 60 + segundos.trim().parse::<u32>().ok().filter(|segundos| *segundos < 60)?;
                    Some((f64::from(total), f64::from(total) + 1.0))
                }
                None => {
                    let minutos = numero(texto)?;
                    Some((minutos * 60.0, minutos * 60.0 + 60.0))
                }
            },
            // El tempo se muestra redondeado: 120 abarca de 119,5 a 120,5.
            CampoNumerico::Bpm => numero(texto).map(|bpm| (bpm - 0.5, bpm + 0.5)),
            _ => texto.trim().parse::<u32>().ok().map(|entero| (f64::from(entero), f64::from(entero) + 1.0)),
        }
    };
    intervalo_de(separar_comparacion(valor, true), leer).ok_or_else(|| {
        let ejemplo = match campo {
            CampoNumerico::Posicion => tr!("#508 o #10..20"),
            CampoNumerico::Anio => tr!("anio:1995, anio:>=2000 o anio:1990..1999"),
            CampoNumerico::Ranking => tr!("ranking:5 o ranking:>=4"),
            CampoNumerico::Bpm => tr!("bpm:120 o bpm:118..126"),
            CampoNumerico::Duracion => tr!("duracion:<3:30 o duracion:3..5 (minutos)"),
        };
        tr!("No se entiende «{valor}» para {}; por ejemplo: {ejemplo}", campo.nombre(), valor = valor, ejemplo = ejemplo)
    })
}

/// Numero con punto o coma decimal.
fn numero(texto: &str) -> Option<f64> {
    texto.trim().replace(',', ".").parse::<f64>().ok().filter(|valor| valor.is_finite() && *valor >= 0.0)
}

fn interpretar_sonado(valor: &str, ahora: &Zoned) -> Result<Criterio, String> {
    let error = || {
        tr!("No se entiende «{valor}» para sonado; por ejemplo: sonado:hoy, sonado:7d, sonado:>30d, sonado:2026-11-03, sonado:<01/10/2026 o sonado:nunca", valor = valor)
    };
    let simplificado = simplificar(valor);
    if simplificado == "nunca" || simplificado == "never" {
        return Ok(Criterio::NuncaSonado);
    }
    let comparacion = separar_comparacion(valor, false);
    // Con un lapso ("7d"): menos de 7 dias es despues de hace 7 dias.
    let lapso = |texto: &str| hace(texto, ahora);
    let relativo = match comparacion {
        Comparacion::Igual(texto) | Comparacion::Menor(texto) | Comparacion::MenorOIgual(texto) => {
            lapso(texto).map(|limite| Intervalo { desde: Some(limite), hasta: None })
        }
        Comparacion::Mayor(texto) | Comparacion::MayorOIgual(texto) => lapso(texto).map(|limite| Intervalo { desde: None, hasta: Some(limite) }),
        Comparacion::Tramo(..) => None,
    };
    if let Some(intervalo) = relativo {
        return Ok(Criterio::Sonado(intervalo));
    }
    let leer = |texto: &str| dias_de(texto, ahora);
    intervalo_de(separar_comparacion(valor, false), leer).map(Criterio::Sonado).ok_or_else(error)
}

/// Instante de hace un lapso ("7d", "12h", "2s" semanas, "3m" meses, "1a" anos).
fn hace(texto: &str, ahora: &Zoned) -> Option<f64> {
    let texto = simplificar(texto);
    let unidad_en = texto.find(|caracter: char| !caracter.is_ascii_digit())?;
    let (cantidad, unidad) = texto.split_at(unidad_en);
    let cantidad: i64 = cantidad.parse().ok().filter(|cantidad| *cantidad <= 10_000)?;
    let lapso = match unidad.trim() {
        "h" | "hora" | "horas" | "hour" | "hours" => Span::new().hours(cantidad),
        "d" | "dia" | "dias" | "day" | "days" => Span::new().days(cantidad),
        "s" | "sem" | "semana" | "semanas" | "w" | "week" | "weeks" => Span::new().weeks(cantidad),
        "m" | "mes" | "meses" | "mo" | "month" | "months" => Span::new().months(cantidad),
        "a" | "ano" | "anos" | "anio" | "anios" | "y" | "year" | "years" => Span::new().years(cantidad),
        _ => return None,
    };
    Some(ahora.checked_sub(lapso).ok()?.timestamp().as_second() as f64)
}

/// Intervalo de un dia (o un mes) local: "hoy", "ayer", "2026-11-03",
/// "03/11/2026", "3/11" (de este ano) o "2026-11" (todo el mes).
fn dias_de(texto: &str, ahora: &Zoned) -> Option<(f64, f64)> {
    let hoy = ahora.date();
    let texto = simplificar(texto);
    let (inicio, fin) = match texto.as_str() {
        "hoy" | "today" => (hoy, hoy.tomorrow().ok()?),
        "ayer" | "yesterday" => (hoy.yesterday().ok()?, hoy),
        _ => {
            if let Ok(dia) = texto.parse::<Date>() {
                (dia, dia.tomorrow().ok()?)
            } else if let Some((anio, mes)) = texto.split_once('-') {
                let inicio = Date::new(anio.parse().ok()?, mes.parse().ok()?, 1).ok()?;
                (inicio, inicio.last_of_month().tomorrow().ok()?)
            } else {
                let partes: Vec<&str> = texto.split('/').collect();
                let (dia, mes, anio) = match partes.as_slice() {
                    [dia, mes] => (dia, mes, hoy.year()),
                    [dia, mes, anio] => (dia, mes, anio.parse::<i16>().ok()?),
                    _ => return None,
                };
                let fecha = Date::new(anio, mes.parse().ok()?, dia.parse().ok()?).ok()?;
                (fecha, fecha.tomorrow().ok()?)
            }
        }
    };
    let zona: TimeZone = ahora.time_zone().clone();
    let segundos = |fecha: Date| -> Option<f64> { Some(fecha.to_zoned(zona.clone()).ok()?.timestamp().as_second() as f64) };
    Some((segundos(inicio)?, segundos(fin)?))
}

fn simplificar(texto: &str) -> String {
    texto.trim().chars().flat_map(char::to_lowercase).map(sin_acento).collect()
}

#[cfg(test)]
mod pruebas {
    #[test]
    fn los_prefijos_en_ingles_valen_igual() {
        let ahora: Zoned = "2026-11-08T15:00:00-03:00[America/Argentina/Buenos_Aires]".parse().unwrap();
        for (espanol, ingles) in [
            ("artista:sanz", "artist:sanz"),
            ("titulo:amiga", "title:amiga"),
            ("ranking:>=4", "rating:>=4"),
            ("anio:1990..1999", "year:1990..1999"),
            ("sonado:hoy", "played:today"),
            ("sonado:nunca", "played:never"),
            ("sonado:>2s", "played:>2w"),
            ("sonado:<1a", "played:<1y"),
        ] {
            let a = ConsultaDeTemas::nueva_en(espanol, &ahora).unwrap().unwrap();
            let b = ConsultaDeTemas::nueva_en(ingles, &ahora).unwrap().unwrap();
            assert_eq!(format!("{a:?}"), format!("{b:?}"), "{espanol} / {ingles}");
        }
    }

    use super::*;

    fn ahora() -> Zoned {
        // Domingo 8 de noviembre de 2026, 15:00 en Buenos Aires (UTC-3).
        "2026-11-08T15:00:00-03:00[America/Argentina/Buenos_Aires]".parse().unwrap()
    }

    fn consulta(texto: &str) -> ConsultaDeTemas {
        ConsultaDeTemas::nueva_en(texto, &ahora()).unwrap().unwrap()
    }

    fn tema<'a>() -> DatosDelTema<'a> {
        DatosDelTema {
            titulo: "Corazón partío",
            artista: "Alejandro Sanz",
            genero: "Pop latino",
            album: "Más",
            tipo: "Tema",
            nota: "Pedido de oyentes",
            archivo: "D:/musica/sanz/corazon.mp3",
            ruta: "D:/musica/sanz/corazon.mp3",
            anio: Some(1997),
            ranking: Some(4),
            bpm: Some(120.3),
            duracion: Some(5.0 * 60.0 + 47.0),
            // 6 de noviembre, 22:40 en Buenos Aires.
            ultima_reproduccion: Some("2026-11-07T01:40:00Z"),
            posicion: Some(508),
        }
    }

    #[test]
    fn sin_prefijos_busca_todas_las_palabras_como_antes() {
        assert!(consulta("  corazon SANZ ").coincide(&tema()));
        assert!(consulta("mas").coincide(&tema()), "el album tambien");
        assert!(!consulta("corazon soda").coincide(&tema()));
        assert!(!consulta("oyentes").coincide(&tema()), "la nota solo con su prefijo");
        assert_eq!(ConsultaDeTemas::nueva_en("   ", &ahora()), Ok(None));
        // Un prefijo desconocido es una palabra mas.
        assert!(!consulta("12:30").coincide(&tema()));
    }

    #[test]
    fn salta_a_una_posicion_o_a_un_tramo() {
        assert!(consulta("#508").coincide(&tema()));
        assert!(!consulta("#507").coincide(&tema()));
        assert!(consulta("#500..510").coincide(&tema()));
        assert!(consulta("#500-510").coincide(&tema()));
        assert!(consulta("#508").usa_la_posicion());
        assert!(!consulta("sanz").usa_la_posicion());
        assert!(!consulta("#508").coincide(&DatosDelTema { posicion: None, ..tema() }), "fuera de la lista no hay posicion");
        assert!(ConsultaDeTemas::nueva_en("#abc", &ahora()).is_err());
    }

    #[test]
    fn busca_en_un_solo_dato_con_su_prefijo() {
        assert!(consulta("artista:sanz").coincide(&tema()));
        assert!(!consulta("titulo:sanz").coincide(&tema()));
        assert!(consulta("a:SANZ t:corazón").coincide(&tema()));
        assert!(consulta("genero:\"pop latino\"").coincide(&tema()));
        assert!(!consulta("genero:\"latino pop\"").coincide(&tema()));
        assert!(consulta("nota:oyentes").coincide(&tema()));
        assert!(consulta("ruta:musica/sanz").coincide(&tema()));
        assert!(consulta("Género:pop").coincide(&tema()), "el prefijo sin importar mayusculas ni acentos");
        assert!(ConsultaDeTemas::nueva_en("artista:", &ahora()).is_err());
    }

    #[test]
    fn compara_los_datos_con_numero() {
        assert!(consulta("ranking:4").coincide(&tema()));
        assert!(consulta("ranking:>=4").coincide(&tema()));
        assert!(!consulta("ranking:>4").coincide(&tema()));
        assert!(consulta("r:<5").coincide(&tema()));
        assert!(consulta("anio:1990..1999").coincide(&tema()));
        assert!(consulta("año:1990-1999").coincide(&tema()));
        assert!(!consulta("anio:>=2000").coincide(&tema()));
        assert!(consulta("anio:..1997").coincide(&tema()));
        assert!(consulta("bpm:120").coincide(&tema()));
        assert!(consulta("bpm:118..122").coincide(&tema()));
        assert!(!consulta("bpm:>120").coincide(&tema()));
        assert!(consulta("duracion:5").coincide(&tema()), "5 minutos abarca de 5:00 a 5:59");
        assert!(consulta("duracion:>5:30").coincide(&tema()));
        assert!(!consulta("duracion:<3:30").coincide(&tema()));
        assert!(!consulta("bpm:120").coincide(&DatosDelTema { bpm: None, ..tema() }), "sin el dato no coincide");
        assert!(ConsultaDeTemas::nueva_en("ranking:mucho", &ahora()).is_err());
        assert!(ConsultaDeTemas::nueva_en("duracion:3:75", &ahora()).is_err());
    }

    #[test]
    fn busca_por_la_ultima_reproduccion() {
        assert!(consulta("sonado:7d").coincide(&tema()), "hace 2 dias");
        assert!(consulta("sonado:<3d").coincide(&tema()));
        assert!(!consulta("sonado:<1d").coincide(&tema()));
        assert!(!consulta("sonado:>30d").coincide(&tema()));
        assert!(consulta("sonado:>12h").coincide(&tema()));
        assert!(consulta("sonado:2026-11-06").coincide(&tema()), "el dia local, no el UTC");
        assert!(!consulta("sonado:2026-11-07").coincide(&tema()));
        assert!(consulta("sonado:06/11/2026").coincide(&tema()));
        assert!(consulta("sonado:6/11").coincide(&tema()));
        assert!(consulta("sonado:2026-11").coincide(&tema()));
        assert!(consulta("sonado:2026-11-01..2026-11-06").coincide(&tema()));
        assert!(consulta("sonado:>=2026-11-06").coincide(&tema()));
        assert!(!consulta("sonado:>2026-11-06").coincide(&tema()));
        assert!(consulta("sonado:<2026-11-07").coincide(&tema()));
        assert!(!consulta("sonado:hoy").coincide(&tema()));
        assert!(!consulta("sonado:ayer").coincide(&tema()));
        let nunca = DatosDelTema { ultima_reproduccion: None, ..tema() };
        assert!(consulta("sonado:nunca").coincide(&nunca));
        assert!(!consulta("sonado:nunca").coincide(&tema()));
        assert!(!consulta("sonado:>30d").coincide(&nunca), "sin reproducir no tiene fecha");
        assert!(consulta("-sonado:<30d").coincide(&nunca), "negada, incluye los nunca reproducidos");
        assert!(ConsultaDeTemas::nueva_en("sonado:pronto", &ahora()).is_err());
    }

    #[test]
    fn el_guion_niega_cualquier_condicion() {
        assert!(consulta("-genero:cumbia").coincide(&tema()));
        assert!(!consulta("-sanz").coincide(&tema()));
        assert!(consulta("sanz -soda ranking:>=3").coincide(&tema()));
        assert!(consulta("-").coincide(&DatosDelTema { titulo: "-", ..DatosDelTema::default() }), "un guion solo es texto");
    }
}
