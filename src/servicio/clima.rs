//! Temperatura y humedad actuales de una localidad, desde la API gratuita de
//! Open-Meteo (no pide clave). Se consultan en un hilo propio cada
//! `INTERVALO_DE_CONSULTA` y se guardan para que el anuncio de hora,
//! temperatura y humedad las tome al instante, sin esperar a la red.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Cada cuanto se vuelve a consultar el clima.
const INTERVALO_DE_CONSULTA: Duration = Duration::from_secs(10 * 60);
/// Espera antes de reintentar una consulta fallida.
const ESPERA_TRAS_UN_FALLO: Duration = Duration::from_secs(2 * 60);
/// Pasado este tiempo sin una consulta buena, el dato ya no se anuncia.
const VIGENCIA_DEL_DATO: Duration = Duration::from_secs(60 * 60);
const TIEMPO_MAXIMO_DE_LA_CONSULTA: Duration = Duration::from_secs(15);
/// Tras descartar una consulta por un cambio de localidad, espera de la
/// localidad nueva (se pide justo despues del cambio).
const ESPERA_DEL_PEDIDO: Duration = Duration::from_secs(1);
/// Localidades que devuelve como maximo una busqueda por nombre.
const LOCALIDADES_POR_BUSQUEDA: usize = 20;

/// Localidad de la que se anuncia el clima.
#[derive(Clone, Debug, PartialEq)]
pub struct Localidad {
    pub nombre: String,
    pub latitud: f64,
    pub longitud: f64,
}

impl Default for Localidad {
    /// Ciudad Autonoma de Buenos Aires.
    fn default() -> Self {
        Self { nombre: "Buenos Aires".into(), latitud: -34.6037, longitud: -58.3816 }
    }
}

/// Localidad encontrada por nombre, con su region (partido o departamento,
/// provincia y pais) para distinguir las homonimas.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalidadEncontrada {
    pub localidad: Localidad,
    pub region: String,
}

/// Lee las localidades de la respuesta del geocodificador de Open-Meteo:
/// `{"results": [{"name": "Cordoba", "latitude": -31.41, "longitude": -64.18,
/// "admin2": "Capital", "admin1": "Cordoba", "country": "Argentina"}]}`.
/// Sin coincidencias la respuesta no trae `results`.
pub fn interpretar_localidades(texto: &str) -> Result<Vec<LocalidadEncontrada>, String> {
    let raiz: Value =
        serde_json::from_str(texto).map_err(|error| tr!("Respuesta de la busqueda de localidades ilegible: {error}", error = error))?;
    if let Some(motivo) = raiz.get("reason").and_then(Value::as_str) {
        return Err(tr!("La busqueda de localidades fallo: {motivo}", motivo = motivo));
    }
    let Some(resultados) = raiz.get("results").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    Ok(resultados
        .iter()
        .filter_map(|resultado| {
            let texto = |clave: &str| resultado.get(clave).and_then(Value::as_str).map(str::trim).filter(|valor| !valor.is_empty());
            let numero = |clave: &str| resultado.get(clave).and_then(Value::as_f64).filter(|valor| valor.is_finite());
            let nombre = texto("name")?;
            let (latitud, longitud) = (numero("latitude")?, numero("longitude")?);
            if !(-90.0..=90.0).contains(&latitud) || !(-180.0..=180.0).contains(&longitud) {
                return None;
            }
            // El partido que se llama como la localidad no se repite; la
            // provincia y el pais siempre van (distinguen las homonimas).
            let mut partes: Vec<&str> = texto("admin2").filter(|partido| *partido != nombre).into_iter().collect();
            for parte in ["admin1", "country"].into_iter().filter_map(texto) {
                if partes.last() != Some(&parte) {
                    partes.push(parte);
                }
            }
            Some(LocalidadEncontrada {
                localidad: Localidad { nombre: nombre.to_string(), latitud, longitud },
                region: partes.join(", "),
            })
        })
        .collect())
}

/// Busca localidades por nombre en el geocodificador gratuito de Open-Meteo
/// (bloquea hasta que responde o vence el tiempo).
pub fn buscar_localidades(nombre: &str) -> Result<Vec<LocalidadEncontrada>, String> {
    let agente: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(TIEMPO_MAXIMO_DE_LA_CONSULTA)).build().into();
    let texto = agente
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query("name", nombre.trim())
        .query("count", LOCALIDADES_POR_BUSQUEDA.to_string())
        .query("language", "es")
        .query("format", "json")
        .call()
        .map_err(|error| tr!("No se pudo buscar la localidad: {error}", error = error))?
        .body_mut()
        .read_to_string()
        .map_err(|error| tr!("No se pudo leer la busqueda de localidades: {error}", error = error))?;
    interpretar_localidades(&texto)
}

/// Temperatura (grados centigrados) y humedad relativa (por ciento).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clima {
    pub temperatura: f64,
    pub humedad: f64,
}

/// Estado del clima para mostrar: si ya se consulta, si fallo la ultima
/// consulta y la ultima lectura buena con su antiguedad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LecturaDeClima {
    pub iniciado: bool,
    pub sin_conexion: bool,
    pub ultima: Option<(Clima, Duration)>,
}

impl LecturaDeClima {
    /// La ultima lectura, si todavia vale para el anuncio.
    pub fn vigente(&self) -> Option<(Clima, Duration)> {
        self.ultima.filter(|(_, antiguedad)| *antiguedad <= VIGENCIA_DEL_DATO)
    }
}

/// Direccion de la consulta del clima actual de la localidad.
fn direccion_de_la_consulta(localidad: &Localidad) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}&current=temperature_2m,relative_humidity_2m&timezone=auto",
        localidad.latitud, localidad.longitud
    )
}

/// Lee temperatura y humedad de la respuesta de Open-Meteo:
/// `{"current": {"temperature_2m": 17.4, "relative_humidity_2m": 76, ...}}`.
pub fn interpretar_respuesta(texto: &str) -> Result<Clima, String> {
    let raiz: Value = serde_json::from_str(texto).map_err(|error| tr!("Respuesta del clima ilegible: {error}", error = error))?;
    let actual = raiz.get("current").ok_or_else(|| tr!("La respuesta del clima no trae el dato actual"))?;
    let numero = |clave: &str| actual.get(clave).and_then(Value::as_f64).filter(|valor| valor.is_finite());
    Ok(Clima {
        temperatura: numero("temperature_2m").ok_or_else(|| tr!("La respuesta del clima no trae la temperatura"))?,
        humedad: numero("relative_humidity_2m").ok_or_else(|| tr!("La respuesta del clima no trae la humedad"))?,
    })
}

/// Consulta el clima actual (bloquea hasta que responde o vence el tiempo).
pub fn consultar(localidad: &Localidad) -> Result<Clima, String> {
    let agente: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(TIEMPO_MAXIMO_DE_LA_CONSULTA)).build().into();
    let texto = agente
        .get(&direccion_de_la_consulta(localidad))
        .call()
        .map_err(|error| tr!("No se pudo consultar el clima: {error}", error = error))?
        .body_mut()
        .read_to_string()
        .map_err(|error| tr!("No se pudo leer la respuesta del clima: {error}", error = error))?;
    interpretar_respuesta(&texto)
}

/// Ultimo clima conocido, que un hilo propio mantiene al dia.
#[derive(Default)]
pub struct ServicioDeClima {
    ultimo: Arc<Mutex<Option<(Clima, Instant)>>>,
    iniciado: AtomicBool,
    // La ultima consulta fallo (sin conexion o respuesta invalida).
    sin_conexion: Arc<AtomicBool>,
    // Para despertar al hilo con otra localidad (o la misma, para consultar
    // en el momento).
    pedidos: Mutex<Option<Sender<Localidad>>>,
    // Sube al cambiar de localidad: la consulta que estaba en curso no pisa
    // el dato de la nueva con el de la anterior.
    generacion: Arc<AtomicU64>,
}

impl ServicioDeClima {
    /// Arranca las consultas periodicas de la localidad (solo la primera vez:
    /// quien no usa el anuncio no sale a la red).
    pub fn iniciar(&self, localidad: Localidad) {
        if self.iniciado.swap(true, Ordering::SeqCst) {
            return;
        }
        let ultimo = Arc::clone(&self.ultimo);
        let sin_conexion = Arc::clone(&self.sin_conexion);
        let generacion = Arc::clone(&self.generacion);
        let (emisor, receptor) = mpsc::channel::<Localidad>();
        if let Ok(mut pedidos) = self.pedidos.lock() {
            *pedidos = Some(emisor);
        }
        let lanzado = std::thread::Builder::new().name("clima".into()).spawn(move || {
            let mut localidad = localidad;
            loop {
                let de_esta_localidad = generacion.load(Ordering::SeqCst);
                let resultado = consultar(&localidad);
                let espera = if generacion.load(Ordering::SeqCst) != de_esta_localidad {
                    // Cambio la localidad mientras se consultaba: se descarta y
                    // se espera el pedido con la nueva, que llega enseguida.
                    ESPERA_DEL_PEDIDO
                } else {
                    match resultado {
                    Ok(clima) => {
                        log::info!(
                            "Clima de {}: {:.1} grados, {:.0} % de humedad",
                            localidad.nombre,
                            clima.temperatura,
                            clima.humedad
                        );
                        if let Ok(mut ultimo) = ultimo.lock() {
                            *ultimo = Some((clima, Instant::now()));
                        }
                        sin_conexion.store(false, Ordering::SeqCst);
                        INTERVALO_DE_CONSULTA
                    }
                    Err(error) => {
                        log::warn!("{error}");
                        sin_conexion.store(true, Ordering::SeqCst);
                        ESPERA_TRAS_UN_FALLO
                    }
                    }
                };
                // Espera la proxima consulta o un pedido (otra localidad o
                // consultar ya); si hay varios, vale el ultimo.
                match receptor.recv_timeout(espera) {
                    Ok(pedida) => localidad = receptor.try_iter().last().unwrap_or(pedida),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        if let Err(error) = lanzado {
            log::warn!("No se pudo iniciar la consulta del clima: {error}");
            self.iniciado.store(false, Ordering::SeqCst);
        }
    }

    /// Pasa a consultar otra localidad: descarta el dato de la anterior y, si
    /// las consultas ya andan, pide la nueva en el momento (si no, se usara al
    /// iniciarlas). Devuelve si las consultas estaban en marcha.
    pub fn cambiar_localidad(&self, localidad: Localidad) -> bool {
        self.generacion.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut ultimo) = self.ultimo.lock() {
            *ultimo = None;
        }
        self.sin_conexion.store(false, Ordering::SeqCst);
        self.pedir(localidad)
    }

    /// Consulta ya, sin esperar el proximo turno. Devuelve si las consultas
    /// estaban en marcha (si no, no hace nada).
    pub fn consultar_ahora(&self, localidad: Localidad) -> bool {
        self.pedir(localidad)
    }

    fn pedir(&self, localidad: Localidad) -> bool {
        self.iniciado.load(Ordering::SeqCst)
            && self.pedidos.lock().ok().and_then(|pedidos| pedidos.as_ref().map(|emisor| emisor.send(localidad).is_ok())).unwrap_or(false)
    }

    /// Lo que se sabe del clima, para mostrarlo.
    pub fn lectura(&self) -> LecturaDeClima {
        let ultima = self.ultimo.lock().ok().and_then(|ultimo| ultimo.map(|(clima, momento)| (clima, momento.elapsed())));
        LecturaDeClima {
            iniciado: self.iniciado.load(Ordering::SeqCst),
            sin_conexion: self.sin_conexion.load(Ordering::SeqCst),
            ultima,
        }
    }

    /// El ultimo clima conocido, si no es demasiado viejo para anunciarlo.
    pub fn vigente(&self) -> Option<Clima> {
        let ultimo = self.ultimo.lock().ok()?;
        ultimo.filter(|(_, momento)| momento.elapsed() <= VIGENCIA_DEL_DATO).map(|(clima, _)| clima)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_temperatura_y_humedad_de_open_meteo() {
        let respuesta = r#"{"latitude":-34.62,"current_units":{"temperature_2m":"°C"},
            "current":{"time":"2026-11-06T21:15","interval":900,"temperature_2m":17.4,"relative_humidity_2m":76}}"#;
        assert_eq!(interpretar_respuesta(respuesta), Ok(Clima { temperatura: 17.4, humedad: 76.0 }));
    }

    #[test]
    fn lee_las_localidades_del_geocodificador_con_su_region() {
        let respuesta = r#"{"results":[
            {"id":1,"name":"Córdoba","latitude":-31.4135,"longitude":-64.18105,"admin1":"Córdoba","admin2":"Capital","country":"Argentina"},
            {"id":2,"name":"Córdoba","latitude":37.89155,"longitude":-4.77275,"admin1":"Andalucía","admin2":"Córdoba","country":"España"},
            {"id":3,"name":"Sin coordenadas","country":"Argentina"},
            {"id":4,"name":"Fuera de rango","latitude":120.0,"longitude":10.0}
        ],"generationtime_ms":0.5}"#;
        let encontradas = interpretar_localidades(respuesta).expect("respuesta valida");
        assert_eq!(encontradas.len(), 2);
        assert_eq!(encontradas[0].localidad, Localidad { nombre: "Córdoba".into(), latitud: -31.4135, longitud: -64.18105 });
        assert_eq!(encontradas[0].region, "Capital, Córdoba, Argentina");
        // El partido con el mismo nombre que la localidad no se repite.
        assert_eq!(encontradas[1].region, "Andalucía, España");
    }

    #[test]
    fn una_busqueda_sin_coincidencias_no_es_un_error() {
        assert_eq!(interpretar_localidades(r#"{"generationtime_ms":0.3}"#), Ok(Vec::new()));
        assert!(interpretar_localidades(r#"{"error":true,"reason":"Parameter count must be between 1 and 100."}"#).is_err());
        assert!(interpretar_localidades("no es json").is_err());
    }

    #[test]
    fn cambiar_de_localidad_descarta_el_dato_de_la_anterior() {
        let servicio = ServicioDeClima::default();
        if let Ok(mut ultimo) = servicio.ultimo.lock() {
            *ultimo = Some((Clima { temperatura: 20.0, humedad: 50.0 }, Instant::now()));
        }
        servicio.sin_conexion.store(true, Ordering::SeqCst);
        // Sin las consultas en marcha no sale a la red: se usara al iniciarlas.
        assert!(!servicio.cambiar_localidad(Localidad::default()));
        let lectura = servicio.lectura();
        assert!(lectura.ultima.is_none() && !lectura.sin_conexion);
        assert!(servicio.vigente().is_none());
        assert!(!servicio.consultar_ahora(Localidad::default()));
    }

    #[test]
    fn una_respuesta_incompleta_es_un_error() {
        assert!(interpretar_respuesta(r#"{"current":{"temperature_2m":17.4}}"#).is_err());
        assert!(interpretar_respuesta(r#"{"error":true,"reason":"Latitude must be in range"}"#).is_err());
        assert!(interpretar_respuesta("no es json").is_err());
    }

    #[test]
    fn la_consulta_pide_el_clima_actual_de_la_localidad() {
        let direccion = direccion_de_la_consulta(&Localidad::default());
        assert!(direccion.contains("latitude=-34.6037&longitude=-58.3816"));
        assert!(direccion.contains("current=temperature_2m,relative_humidity_2m"));
    }

    #[test]
    fn sin_consultas_no_hay_clima_vigente() {
        assert_eq!(ServicioDeClima::default().vigente(), None);
    }

    /// Consulta real a Open-Meteo (necesita internet): cargo test -- --ignored clima_real
    #[test]
    #[ignore]
    fn clima_real() {
        let clima = consultar(&Localidad::default()).expect("consulta");
        println!("{clima:?}");
        assert!((-30.0..=55.0).contains(&clima.temperatura));
        assert!((0.0..=100.0).contains(&clima.humedad));
    }
}
