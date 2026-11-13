/// Catalogo de traducciones al inglés: lo usan la interfaz Slint y la macro
/// `tr!` del codigo Rust.
const CATALOGO_EN_INGLES: &str = "lang/en/LC_MESSAGES/mcp-raptor-player.po";

fn main() {
    // La interfaz usa siempre el tema oscuro, sin importar el del sistema. Los
    // textos marcados con @tr() se traducen con los catalogos de lang/, sin
    // contexto (el mismo texto se traduce igual en todas las ventanas).
    let configuracion = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/ventana.slint", configuracion).expect("No se pudo compilar la interfaz Slint");
    generar_traducciones_para_rust();
    embeber_recursos_de_windows();
}

/// Escribe en OUT_DIR la macro `traduccion_en!`, que lleva cada texto en
/// español a su traduccion: asi `format!` recibe la plantilla traducida como
/// literal y el compilador verifica sus marcadores. Un texto sin traduccion
/// queda en español.
fn generar_traducciones_para_rust() {
    println!("cargo:rerun-if-changed={CATALOGO_EN_INGLES}");
    let catalogo = std::fs::read_to_string(CATALOGO_EN_INGLES).unwrap_or_default();
    let mut macro_rust = String::from("macro_rules! traduccion_en {\n");
    for (espanol, ingles) in leer_po(&catalogo) {
        if !espanol.is_empty() && !ingles.is_empty() {
            // El contenido del .po usa los mismos escapes que un literal de Rust.
            macro_rust.push_str(&format!("    (\"{espanol}\") => {{ \"{ingles}\" }};\n"));
        }
    }
    macro_rust.push_str("    ($otro:literal) => { $otro };\n}\n");
    let destino = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("traducciones.rs");
    std::fs::write(destino, macro_rust).expect("No se pudieron escribir las traducciones");
}

/// Pares (msgid, msgstr) de un catalogo .po, con el contenido tal como esta
/// escrito (sin quitar los escapes). No usa contexto ni plurales.
fn leer_po(texto: &str) -> Vec<(String, String)> {
    let mut entradas = Vec::new();
    let (mut id, mut traduccion, mut campo) = (None::<String>, String::new(), 0);
    let cadena = |resto: &str| {
        let resto = resto.trim();
        resto.strip_prefix('"').and_then(|resto| resto.strip_suffix('"')).unwrap_or(resto).to_string()
    };
    for linea in texto.lines().map(str::trim) {
        if let Some(resto) = linea.strip_prefix("msgid ") {
            if let Some(anterior) = id.take() {
                entradas.push((anterior, std::mem::take(&mut traduccion)));
            }
            id = Some(cadena(resto));
            campo = 1;
        } else if let Some(resto) = linea.strip_prefix("msgstr ") {
            traduccion = cadena(resto);
            campo = 2;
        } else if linea.starts_with('"') {
            match (campo, id.as_mut()) {
                (1, Some(id)) => id.push_str(&cadena(linea)),
                (2, _) => traduccion.push_str(&cadena(linea)),
                _ => {}
            }
        }
    }
    if let Some(anterior) = id {
        entradas.push((anterior, traduccion));
    }
    entradas
}

/// En Windows, el icono del .exe (el que usan el Explorador, los accesos
/// directos y la barra de tareas al fijar el programa) y la descripcion que se
/// muestra como nombre. El icono de la ventana abierta lo pone Slint.
#[cfg(windows)]
fn embeber_recursos_de_windows() {
    println!("cargo:rerun-if-changed=recursos/raptor.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut recursos = winresource::WindowsResource::new();
    recursos
        .set_icon("recursos/raptor.ico")
        .set("FileDescription", "MCP Raptor Player")
        .set("ProductName", "MCP Raptor Player");
    recursos.compile().expect("No se pudieron embeber el icono y los datos del programa");
}

#[cfg(not(windows))]
fn embeber_recursos_de_windows() {}
