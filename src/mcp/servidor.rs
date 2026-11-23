//! Servidor MCP (Model Context Protocol) embebido en el reproductor.
//!
//! Implementa el transporte "Streamable HTTP" en su forma minima: cada POST a
//! /mcp lleva un mensaje JSON-RPC y se responde con un unico JSON, sin
//! sesiones ni flujos SSE. Solo escucha en la interfaz local (127.0.0.1) y
//! rechaza peticiones con un Origin ajeno, para que una pagina web no pueda
//! manipular la lista.

use std::sync::Arc;
use std::thread;

use serde_json::{Map, Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use super::herramientas;

pub const VARIABLE_DE_PUERTO: &str = "RAPTOR_MCP_PUERTO";
pub const PUERTO_POR_DEFECTO: u16 = 8765;

const RUTA: &str = "/mcp";
const VERSION_PROTOCOLO: &str = "2025-11-25";
const VERSIONES_SOPORTADAS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// Icono del conector (la R de Raptor), que los clientes muestran junto a su
/// nombre: PNG de 64 px (`cargo test generar_icono_del_mcp -- --ignored`) y
/// el SVG de la ventana.
const ICONO_PNG: &[u8] = include_bytes!("../../recursos/raptor-mcp.png");
const ICONO_SVG: &[u8] = include_bytes!("../../ui/iconos/ventana-raptor.svg");
const HOSTS_LOCALES: &[&str] = &["localhost", "127.0.0.1", "[::1]"];

const METODO_NO_ENCONTRADO: i64 = -32601;
const PARAMETROS_INVALIDOS: i64 = -32602;
const PETICION_INVALIDA: i64 = -32600;
const ERROR_DE_PARSEO: i64 = -32700;

pub struct ServidorMcp {
    servidor: Arc<Server>,
}

impl ServidorMcp {
    pub fn iniciar(puerto: u16) -> Result<Self, String> {
        let servidor = Arc::new(Server::http(("127.0.0.1", puerto)).map_err(|error| error.to_string())?);
        let escucha = servidor.clone();
        thread::Builder::new()
            .name("servidor-mcp".into())
            .spawn(move || {
                for peticion in escucha.incoming_requests() {
                    atender(peticion);
                }
            })
            .map_err(|error| error.to_string())?;
        log::info!("Servidor MCP escuchando en http://127.0.0.1:{puerto}{RUTA}");
        Ok(Self { servidor })
    }
}

impl Drop for ServidorMcp {
    fn drop(&mut self) {
        self.servidor.unblock();
    }
}

fn atender(mut peticion: Request) {
    let origen = peticion
        .headers()
        .iter()
        .find(|encabezado| encabezado.field.equiv("Origin"))
        .map(|encabezado| encabezado.value.as_str().to_string());
    let ruta = peticion.url().split('?').next().unwrap_or_default().to_string();

    let respuesta = if !origen_permitido(origen.as_deref()) {
        Response::empty(403).boxed()
    } else if ruta != RUTA && !ruta.starts_with("/mcp/") {
        Response::empty(404).boxed()
    } else if *peticion.method() != Method::Post {
        // Sin flujo SSE ni sesiones: GET y DELETE no estan soportados.
        Response::empty(405).with_header(encabezado("Allow", "POST")).boxed()
    } else {
        let mut cuerpo = String::new();
        match peticion.as_reader().read_to_string(&mut cuerpo) {
            Err(_) => Response::empty(400).boxed(),
            Ok(_) => match procesar(&cuerpo) {
                None => Response::empty(202).boxed(),
                Some(respuesta) => Response::from_string(respuesta.to_string())
                    .with_header(encabezado("Content-Type", "application/json; charset=utf-8"))
                    .boxed(),
            },
        }
    };
    if let Err(error) = peticion.respond(respuesta) {
        log::warn!("No se pudo responder al cliente MCP: {error}");
    }
}

fn encabezado(campo: &str, valor: &str) -> Header {
    Header::from_bytes(campo.as_bytes(), valor.as_bytes()).expect("encabezado valido")
}

fn origen_permitido(origen: Option<&str>) -> bool {
    let Some(origen) = origen else {
        return true;
    };
    host_de_origen(origen).is_some_and(|host| HOSTS_LOCALES.contains(&host.as_str()))
}

/// Host de un encabezado Origin ("http://localhost:3000" -> "localhost",
/// "http://[::1]:3000" -> "[::1]").
fn host_de_origen(origen: &str) -> Option<String> {
    let (_, resto) = origen.split_once("://")?;
    let autoridad = resto.split(['/', '?', '#']).next()?;
    let autoridad = autoridad.rsplit_once('@').map_or(autoridad, |(_, host)| host);
    let host = if autoridad.starts_with('[') {
        &autoridad[..=autoridad.find(']')?]
    } else {
        autoridad.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// Procesa un mensaje JSON-RPC y devuelve la respuesta, o None si el mensaje
/// era una notificacion (no lleva id y no se responde).
fn procesar(cuerpo: &str) -> Option<Value> {
    let mensaje: Value = match serde_json::from_str(cuerpo) {
        Ok(mensaje) => mensaje,
        Err(_) => return Some(error(Value::Null, ERROR_DE_PARSEO, "JSON invalido")),
    };
    let Some(mensaje) = mensaje.as_object() else {
        return Some(error(Value::Null, PETICION_INVALIDA, "Se esperaba un unico mensaje JSON-RPC"));
    };
    let id = match mensaje.get("id") {
        None | Some(Value::Null) => return None,
        Some(id) => id.clone(),
    };
    let Some(metodo) = mensaje.get("method").and_then(Value::as_str) else {
        return Some(error(id, PETICION_INVALIDA, "Falta el metodo"));
    };
    let vacio = Map::new();
    let parametros = mensaje.get("params").and_then(Value::as_object).unwrap_or(&vacio);

    Some(match metodo {
        "initialize" => resultado(id, inicializar(parametros)),
        "ping" => resultado(id, json!({})),
        "tools/list" => resultado(id, herramientas::definiciones()),
        "tools/call" => llamar_herramienta(id, parametros),
        otro => error(id, METODO_NO_ENCONTRADO, &format!("Metodo no soportado: {otro}")),
    })
}

fn inicializar(parametros: &Map<String, Value>) -> Value {
    let version = parametros
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|pedida| VERSIONES_SOPORTADAS.contains(pedida))
        .unwrap_or(VERSION_PROTOCOLO);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "mcp-raptor-player",
            "title": "MCP Raptor Player",
            "version": env!("CARGO_PKG_VERSION"),
            "websiteUrl": env!("CARGO_PKG_REPOSITORY"),
            "icons": iconos(),
        },
        "instructions": "Controla la lista de reproduccion del MCP Raptor Player abierto en este equipo. \
                         Usa listar_pistas para ver la lista antes de quitar pistas.",
    })
}

/// Los iconos del servidor, como data URI (no dependen de internet).
fn iconos() -> Value {
    json!([
        { "src": format!("data:image/png;base64,{}", base64(ICONO_PNG)), "mimeType": "image/png", "sizes": ["64x64"] },
        { "src": format!("data:image/svg+xml;base64,{}", base64(ICONO_SVG)), "mimeType": "image/svg+xml", "sizes": ["any"] },
    ])
}

/// Base64 estandar (RFC 4648, con relleno).
fn base64(datos: &[u8]) -> String {
    const ALFABETO: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut texto = String::with_capacity(datos.len().div_ceil(3) * 4);
    for grupo in datos.chunks(3) {
        let bytes = [grupo[0], grupo.get(1).copied().unwrap_or(0), grupo.get(2).copied().unwrap_or(0)];
        let valor = u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2]);
        for posicion in 0..4 {
            if posicion <= grupo.len() {
                texto.push(ALFABETO[(valor >> (18 - 6 * posicion) & 0x3f) as usize] as char);
            } else {
                texto.push('=');
            }
        }
    }
    texto
}

fn llamar_herramienta(id: Value, parametros: &Map<String, Value>) -> Value {
    let Some(nombre) = parametros.get("name").and_then(Value::as_str) else {
        return error(id, PARAMETROS_INVALIDOS, "Falta el nombre de la herramienta");
    };
    if !herramientas::existe(nombre) {
        return error(id, PARAMETROS_INVALIDOS, &format!("Herramienta desconocida: {nombre}"));
    }
    let vacio = Map::new();
    let argumentos = parametros.get("arguments").and_then(Value::as_object).unwrap_or(&vacio);
    resultado(id, herramientas::ejecutar(nombre, argumentos))
}

fn resultado(id: Value, contenido: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": contenido })
}

fn error(id: Value, codigo: i64, mensaje: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": codigo, "message": mensaje } })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn acepta_solo_origenes_locales() {
        assert!(origen_permitido(None));
        assert!(origen_permitido(Some("http://localhost:3000")));
        assert!(origen_permitido(Some("http://[::1]:8080")));
        assert!(origen_permitido(Some("https://127.0.0.1")));
        assert!(!origen_permitido(Some("https://ejemplo.com")));
        assert!(!origen_permitido(Some("http://localhost.ejemplo.com")));
        assert!(!origen_permitido(Some("basura")));
    }

    #[test]
    fn las_notificaciones_no_se_responden() {
        assert!(procesar(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn negocia_la_version_del_protocolo() {
        let respuesta = procesar(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        )
        .unwrap();
        assert_eq!(respuesta["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(respuesta["result"]["serverInfo"]["name"], "mcp-raptor-player");
        let icono = respuesta["result"]["serverInfo"]["icons"][0]["src"].as_str().unwrap();
        assert!(icono.starts_with("data:image/png;base64,iVBORw0KGgo"), "un PNG en base64");
        let nueva = procesar(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#).unwrap();
        assert_eq!(nueva["result"]["protocolVersion"], "2025-11-25");
        let otra = procesar(r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#).unwrap();
        assert_eq!(otra["result"]["protocolVersion"], VERSION_PROTOCOLO, "una desconocida recibe la mas nueva");
    }

    #[test]
    fn codifica_en_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn informa_errores_de_protocolo() {
        assert_eq!(procesar("{").unwrap()["error"]["code"], ERROR_DE_PARSEO);
        assert_eq!(procesar(r#"{"id":1,"method":"x"}"#).unwrap()["error"]["code"], METODO_NO_ENCONTRADO);
        let desconocida = procesar(r#"{"id":2,"method":"tools/call","params":{"name":"nada"}}"#).unwrap();
        assert_eq!(desconocida["error"]["code"], PARAMETROS_INVALIDOS);
    }
}
