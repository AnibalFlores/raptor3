//! Servicio HTH (hora, temperatura y humedad) en la lista de reproduccion.
//!
//! El boton HTH de la botonera inserta en la lista una pista especial que
//! apunta a un WAV de la carpeta de datos. Ese archivo se vuelve a armar
//! justo antes de que suene, con las locuciones pregrabadas, la hora del
//! reloj y el ultimo clima conocido: asi dice la hora en que sale al aire y
//! no la de cuando se inserto.
//!
//! La pista no tiene ficha en el catalogo (no se analiza, no se edita y no
//! cuenta pases): suena sin mezcla, como un tipo sin mezcla, y se pinta con
//! su propia franja.
//!
//! Si la carpeta de locuciones tiene bases musicales (`bases`), el boton
//! inserta una base en su lugar: es un tema comun (se analiza, se edita y se
//! mezcla con el anterior y el siguiente) y, al empezar a sonar, la voz del
//! anuncio se arma con la hora en que se abre su ventana de overlay y suena
//! ahi como su efecto fijado, con la musica atenuada.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use slint::ComponentHandle;

use super::{Controlador, archivo_existe, con_controlador};
use crate::VentanaPrincipal;
use crate::audio::Decodificador;
use crate::dominio::{Pista, nombre_de_archivo};
use crate::persistencia::carpeta_del_usuario;
use crate::servicio::{LecturaDeClima, Localidad, Locuciones, armar_anuncio_hth, elegir_base_hth, piezas_del_anuncio, resumen_del_anuncio};

/// Como se muestra la pista en la lista.
pub(super) fn titulo_del_anuncio_hth() -> String {
    tr!("Hora, temperatura y humedad")
}
/// Lo que muestra la columna Tipo para el anuncio y sus bases.
pub(super) const TIPO_DEL_ANUNCIO_HTH: &str = "HTH";
/// Franja de la fila (los tipos usan del 1 al 5).
pub(super) const FRANJA_DEL_ANUNCIO_HTH: i32 = 6;

const NOMBRE_DEL_ARCHIVO: &str = "Hora, temperatura y humedad.wav";
/// La voz que se dice sobre las bases.
const NOMBRE_DE_LA_VOZ: &str = "Voz de hora, temperatura y humedad.wav";
/// Lo que dura la voz si todavia no se armo ninguna (hora, minutos y clima).
const DURACION_TIPICA_DE_LA_VOZ: f64 = 9.0;

/// Archivo de la voz que suena sobre las bases.
fn voz_sobre_las_bases() -> PathBuf {
    carpeta_del_usuario().join("servicios").join(NOMBRE_DE_LA_VOZ)
}

/// Si el archivo es una base musical del anuncio (esta en `bases`).
pub(super) fn es_base_hth(archivo: &Path, carpeta_de_bases: Option<&Path>) -> bool {
    let Some(carpeta_de_bases) = carpeta_de_bases else {
        return false;
    };
    archivo.parent().is_some_and(|carpeta| Pista::new(carpeta) == Pista::new(carpeta_de_bases))
}

/// Si el archivo es una base del anuncio, con la carpeta de bases ya buscada
/// (None si no hay bases en la lista o no se busco).
pub(super) fn es_base_hth_en(archivo: &Path, carpeta_de_bases: Option<&Path>) -> bool {
    podria_ser_base(archivo) && es_base_hth(archivo, carpeta_de_bases)
}

/// Si alguna pista podria ser una base (esta en una carpeta "bases"): evita
/// buscar la carpeta de locuciones en cada refresco de una lista sin bases.
fn podria_ser_base(archivo: &Path) -> bool {
    archivo
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|nombre| nombre.to_string_lossy().eq_ignore_ascii_case("bases"))
}

/// La pista del anuncio: siempre el mismo archivo, en la carpeta de datos.
pub(super) fn pista_del_anuncio_hth() -> &'static Pista {
    static PISTA: OnceLock<Pista> = OnceLock::new();
    PISTA.get_or_init(|| Pista::new(carpeta_del_usuario().join("servicios").join(NOMBRE_DEL_ARCHIVO)))
}

/// Si la ruta es la del anuncio HTH guardado con otra carpeta de datos (otro
/// equipo, otro usuario u otra RAPTOR_CARPETA_DE_DATOS), la del anuncio en
/// esta: carpeta "servicios" y el nombre del archivo del anuncio.
pub(super) fn anuncio_hth_en_esta_carpeta(ruta: &Path) -> Option<PathBuf> {
    let (carpeta, nombre) = super::marcadores::carpeta_y_nombre(ruta)?;
    (carpeta.eq_ignore_ascii_case("servicios") && nombre == NOMBRE_DEL_ARCHIVO).then(|| pista_del_anuncio_hth().archivo().to_path_buf())
}

/// Si el archivo es el del anuncio HTH.
pub(super) fn es_anuncio_hth(archivo: &Path) -> bool {
    let anuncio = pista_del_anuncio_hth();
    // Descarte rapido por el nombre: se pregunta por cada fila de la lista.
    archivo.file_name() == anuncio.archivo().file_name() && Pista::new(archivo) == *anuncio
}

/// Carpetas donde se buscan las locuciones si no se eligio ninguna: junto al
/// programa y en la carpeta de datos.
fn carpetas_habituales() -> Vec<PathBuf> {
    let junto_al_programa =
        std::env::current_exe().ok().and_then(|programa| programa.parent().map(|carpeta| carpeta.join("locuciones")));
    junto_al_programa.into_iter().chain([carpeta_del_usuario().join("locuciones")]).collect()
}

/// Numero con coma decimal, como se escribe en la Argentina: "17,4".
fn con_coma(valor: f64, decimales: usize) -> String {
    format!("{valor:.decimales$}").replace('.', ",")
}

/// Antiguedad en palabras cortas: "recien", "hace 40 s", "hace 3 min",
/// "hace 1 h 5 min".
fn hace_cuanto(antiguedad: Duration) -> String {
    let segundos = antiguedad.as_secs();
    match segundos {
        0..5 => "recien".into(),
        5..60 => tr!("hace {segundos} s", segundos = segundos),
        60..3600 => tr!("hace {} min", segundos / 60),
        _ if segundos % 3600 / 60 == 0 => tr!("hace {} h", segundos / 3600),
        _ => tr!("hace {} h {} min", segundos / 3600, segundos % 3600 / 60),
    }
}

/// Texto del clima bajo el boton HTH y si es "sin datos". La lectura vale
/// mientras sirve para el anuncio; sin ella (sin conexion o demasiado vieja)
/// se informa "sin datos", con la antiguedad de la ultima si la hubo.
pub(super) fn resumen_del_clima(localidad: &str, lectura: &LecturaDeClima) -> (String, bool) {
    match (lectura.vigente(), lectura.ultima) {
        (Some((clima, antiguedad)), _) => {
            let mut texto = tr!("{localidad}: {} °C, humedad {} %\nUltima lectura {}", con_coma(clima.temperatura, 1), con_coma(clima.humedad, 0), hace_cuanto(antiguedad), localidad = localidad);
            if lectura.sin_conexion {
                texto.push_str(&tr!(" (sin conexion: reintentando)"));
            }
            (texto, false)
        }
        (None, Some((_, antiguedad))) => {
            (tr!("{localidad}: sin datos\nSin conexion; ultima lectura {}", hace_cuanto(antiguedad), localidad = localidad), true)
        }
        (None, None) if lectura.sin_conexion => (tr!("{localidad}: sin datos\nSin conexion con el servicio del clima", localidad = localidad), true),
        (None, None) if lectura.iniciado => (tr!("{localidad}: consultando el clima...", localidad = localidad), true),
        (None, None) => (tr!("{localidad}: sin datos\nSe consulta al usar el anuncio HTH", localidad = localidad), true),
    }
}

/// El clima en una linea, para el reloj; vacio si todavia no se pidio.
pub(super) fn clima_para_el_reloj(localidad: &str, lectura: &LecturaDeClima) -> String {
    match (lectura.vigente(), lectura.ultima) {
        (Some((clima, _)), _) => {
            tr!("{} °C · humedad {} % · {localidad}", con_coma(clima.temperatura, 1), con_coma(clima.humedad, 0), localidad = localidad)
        }
        (None, Some(_)) => tr!("{localidad}: sin datos del clima", localidad = localidad),
        (None, None) if lectura.sin_conexion => tr!("{localidad}: sin datos del clima", localidad = localidad),
        (None, None) if lectura.iniciado => tr!("{localidad}: consultando el clima...", localidad = localidad),
        (None, None) => String::new(),
    }
}

impl Controlador {
    /// Locuciones a usar: las de la carpeta elegida o, si no sirve, las de
    /// una carpeta habitual. None si no hay ninguna completa.
    pub(super) fn locuciones(&self) -> Option<Locuciones> {
        let elegida = self.repositorio_de_preferencias.carpeta_de_locuciones().map(Path::to_path_buf);
        elegida.into_iter().chain(carpetas_habituales()).map(Locuciones::new).find(Locuciones::sirve)
    }

    /// Fija la carpeta de las locuciones, si tiene todas las de la hora.
    pub fn elegir_carpeta_de_locuciones(&mut self, carpeta: &Path) -> Result<String, String> {
        let faltantes = Locuciones::new(carpeta).faltantes();
        if !faltantes.is_empty() {
            let muestra: Vec<&str> = faltantes.iter().take(5).map(String::as_str).collect();
            return Err(tr!("En {} faltan {} locucion(es) de la hora, por ejemplo: {}. Se esperan las carpetas hora (00 a 23) y minutos (00 a 59), y opcionalmente temperatura y humedad.", carpeta.display(), faltantes.len(), muestra.join(", ")));
        }
        self.repositorio_de_preferencias.guardar_carpeta_de_locuciones(carpeta);
        Ok(tr!("Locuciones del anuncio HTH: {}", carpeta.display()))
    }

    /// Empieza a consultar el clima de la localidad elegida (una sola vez).
    pub(super) fn iniciar_clima(&self) {
        self.clima.iniciar(self.repositorio_de_preferencias.localidad_del_clima().clone());
    }

    /// Pasa el clima del anuncio a otra localidad: la guarda, descarta el dato
    /// de la anterior y la consulta en el momento (si las consultas no
    /// andaban, las empieza).
    pub(super) fn cambiar_localidad_del_clima(&mut self, localidad: Localidad) {
        self.repositorio_de_preferencias.guardar_localidad_del_clima(localidad.clone());
        if !self.clima.cambiar_localidad(localidad) {
            self.iniciar_clima();
        }
        self.refrescar_clima();
    }

    /// Menu del boton HTH: consulta el clima ya, sin esperar el proximo turno.
    pub(super) fn actualizar_clima_ahora(&mut self) {
        let localidad = self.repositorio_de_preferencias.localidad_del_clima().clone();
        let nombre = localidad.nombre.clone();
        if !self.clima.consultar_ahora(localidad) {
            self.iniciar_clima();
        }
        self.refrescar_clima();
        self.mostrar_estado(&tr!("Consultando el clima de {nombre}...", nombre = nombre));
    }

    /// Al abrir el programa: si ya hay locuciones para el anuncio, empieza a
    /// consultar el clima, para mostrarlo bajo el boton HTH. Sin locuciones
    /// no sale a la red hasta que se use el anuncio.
    /// Muestra bajo el boton HTH la localidad, la temperatura, la humedad y
    /// la antiguedad de la ultima lectura (o "sin datos").
    pub(super) fn refrescar_clima(&self) {
        let localidad = &self.repositorio_de_preferencias.localidad_del_clima().nombre;
        let lectura = self.clima.lectura();
        let (texto, sin_datos) = resumen_del_clima(localidad, &lectura);
        let del_reloj = clima_para_el_reloj(localidad, &lectura);
        self.con_ventana(|ventana| {
            if ventana.get_clima_hth() != texto.as_str() {
                ventana.set_clima_hth(texto.into());
                ventana.set_clima_hth_sin_datos(sin_datos);
            }
            if ventana.get_clima_del_reloj() != del_reloj.as_str() {
                ventana.set_clima_del_reloj(del_reloj.into());
            }
        });
    }

    /// Carpeta de las bases, si alguna pista de la lista esta en una carpeta
    /// "bases" y las locuciones la tienen.
    pub(super) fn carpeta_de_bases_en_la_lista(&self) -> Option<PathBuf> {
        if !self.lista.pistas().iter().any(|pista| podria_ser_base(pista.archivo())) {
            return None;
        }
        self.locuciones().map(|locuciones| locuciones.carpeta_de_bases())
    }

    /// Si la pista es una base del anuncio.
    pub(super) fn es_base_del_anuncio_hth(&self, pista: &Pista) -> bool {
        podria_ser_base(pista.archivo())
            && es_base_hth(pista.archivo(), self.locuciones().map(|locuciones| locuciones.carpeta_de_bases()).as_deref())
    }

    /// Inserta el anuncio en la posicion indicada (desde 0) o, con None, en
    /// el punto de insercion (despues de la seleccionada). Con bases
    /// musicales, inserta una base (al azar, sin repetir la ultima); sin
    /// ellas, la voz sola.
    pub fn insertar_anuncio_hth(&mut self, posicion: Option<usize>) -> Result<String, String> {
        self.iniciar_clima();
        let bases = self.locuciones().map(|locuciones| locuciones.bases()).unwrap_or_default();
        let azar = &mut self.azar_de_locuciones;
        if let Some(base) = elegir_base_hth(&bases, self.ultima_base_hth.as_deref(), &mut |cantidad| azar.elegir(cantidad)).cloned() {
            let posicion = posicion.unwrap_or_else(|| self.punto_de_insercion()).min(self.lista.tamano());
            self.insertar_pistas(vec![base.clone()], Some(posicion), true);
            let mensaje = tr!("Se inserto el anuncio de hora, temperatura y humedad con la base {} en la posicion {}: la voz se dice en su ventana de overlay", nombre_de_archivo(&base), posicion + 1);
            self.ultima_base_hth = Some(base);
            self.mostrar_estado(&mensaje);
            return Ok(mensaje);
        }
        // Se arma ya, para que el archivo exista y se conozca su duracion;
        // al salir al aire se vuelve a armar con la hora de ese momento.
        self.armar_anuncio_hth()?;
        let posicion = posicion.unwrap_or_else(|| self.punto_de_insercion()).min(self.lista.tamano());
        self.insertar_pistas(vec![pista_del_anuncio_hth().archivo().to_path_buf()], Some(posicion), true);
        let mensaje = tr!("Se inserto el anuncio de hora, temperatura y humedad en la posicion {}", posicion + 1);
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Arma el anuncio con la hora actual y el ultimo clima conocido.
    /// Devuelve lo que dice.
    fn armar_anuncio_hth(&mut self) -> Result<String, String> {
        let (resumen, duracion) = self.armar_voz_hth(pista_del_anuncio_hth().archivo(), 0.0)?;
        self.duracion_del_anuncio_hth = Some(duracion);
        Ok(resumen)
    }

    /// Arma la voz en `destino` con la hora que sera dentro de `adelanto`
    /// segundos. Devuelve lo que dice y lo que dura.
    fn armar_voz_hth(&mut self, destino: &Path, adelanto: f64) -> Result<(String, f64), String> {
        let locuciones = self.locuciones().ok_or_else(|| {
            tr!("No se encontraron las locuciones del anuncio HTH: elija la carpeta con el boton de la carpeta, junto al boton HTH.")
        })?;
        // La hora que se anuncia es la del reloj: la de la localidad elegida.
        let ahora = self.ahora_en_la_localidad();
        let momento = jiff::SignedDuration::try_from_secs_f64(adelanto.max(0.0))
            .ok()
            .and_then(|lapso| ahora.checked_add(lapso).ok())
            .unwrap_or(ahora);
        let (hora, minutos) = (momento.hour() as u8, momento.minute() as u8);
        let clima = self.clima.vigente();
        let piezas = piezas_del_anuncio(hora, minutos, clima);
        let azar = &mut self.azar_de_locuciones;
        let armado = armar_anuncio_hth(&locuciones, &piezas, &mut |cantidad| azar.elegir(cantidad), destino)?;
        let resumen = resumen_del_anuncio(hora, minutos, clima);
        log::info!("Anuncio HTH armado ({resumen}, {:.1} s, en {adelanto:.1} s): {:?}", armado.duracion, armado.tomas);
        Ok((resumen, armado.duracion))
    }

    /// Al empezar a sonar una base del anuncio: arma la voz con la hora en que
    /// se abre su ventana de overlay. None si la pista no es una base;
    /// Some(None) si es una base pero no se pudo armar la voz.
    pub(super) fn voz_para_la_base_hth(&mut self, pista: &Pista) -> Option<Option<PathBuf>> {
        if !self.es_base_del_anuncio_hth(pista) {
            return None;
        }
        self.iniciar_clima();
        let ventana = self.lista.puntos_de(pista).overlay();
        let adelanto = ventana.map_or(0.0, |ventana| (ventana.inicio() - self.gestor.tiempo_transcurrido()).max(0.0));
        let voz = voz_sobre_las_bases();
        match self.armar_voz_hth(&voz, adelanto) {
            Ok((resumen, duracion)) => {
                self.duracion_de_la_voz_hth = Some(duracion);
                self.overlay.renovar_perfil(&voz);
                // La base siempre baja mientras se dice la voz.
                self.overlay.atenuar_siempre(&voz);
                let mut estado = tr!("Anuncio HTH sobre {}: {resumen}", nombre_de_archivo(pista.archivo()), resumen = resumen);
                if let Some(aviso) = aviso_de_ventana(ventana.map(|ventana| ventana.fin() - ventana.inicio()), duracion) {
                    estado.push_str(&format!(" ({aviso})"));
                }
                self.mostrar_estado(&estado);
                Some(Some(voz))
            }
            Err(error) => {
                log::warn!("No se pudo armar la voz del anuncio HTH: {error}");
                self.mostrar_estado(&tr!("La base {} suena sin la voz del HTH: {error}", nombre_de_archivo(pista.archivo()), error = error));
                Some(None)
            }
        }
    }

    /// Nombres de las bases para el menu de las filas HTH (solo si hay
    /// alguna fila HTH en la lista).
    pub(super) fn refrescar_bases_hth(&self, hay_filas_hth: bool) {
        let nombres: Vec<slint::SharedString> = if hay_filas_hth {
            self.locuciones()
                .map(|locuciones| locuciones.bases())
                .unwrap_or_default()
                .iter()
                .map(|base| nombre_de_archivo(base).into())
                .collect()
        } else {
            Vec::new()
        };
        self.con_ventana(|ventana| ventana.set_bases_hth(slint::ModelRc::new(slint::VecModel::from(nombres))));
    }

    /// Cambia la base del anuncio HTH de una fila (desde 0) por otra de las
    /// bases (desde 0, en orden de nombre) o, con None, lo deja sin base (la
    /// voz sola). No se toca el que esta sonando.
    pub fn cambiar_base_hth(&mut self, indice: usize, base: Option<usize>) -> Result<String, String> {
        let pista = self
            .lista
            .pista_en(indice)
            .cloned()
            .ok_or_else(|| tr!("Posicion {} fuera de la lista (1 a {}).", indice + 1, self.lista.tamano()))?;
        let es_anuncio = es_anuncio_hth(pista.archivo());
        if !es_anuncio && !self.es_base_del_anuncio_hth(&pista) {
            return Err(tr!("La posicion {} no es un anuncio de hora, temperatura y humedad.", indice + 1));
        }
        if self.gestor.hay_pista_cargada() && self.lista.indice_actual() == Some(indice) {
            return Err(tr!("El anuncio HTH esta sonando: su base se puede cambiar cuando termine.").into());
        }
        let nuevo = match base {
            Some(posicion) => {
                let bases = self.locuciones().map(|locuciones| locuciones.bases()).unwrap_or_default();
                bases.get(posicion).cloned().ok_or_else(|| tr!("No existe la base {} (hay {}).", posicion + 1, bases.len()))?
            }
            None => {
                // La voz sola tiene que existir para quedar en la lista.
                self.armar_anuncio_hth()?;
                pista_del_anuncio_hth().archivo().to_path_buf()
            }
        };
        if Pista::new(&nuevo) == pista {
            return Ok(tr!("El anuncio HTH ya tenia esa base.").into());
        }
        self.quitar_posiciones(&[indice]);
        self.insertar_pistas(vec![nuevo.clone()], Some(indice), true);
        let mensaje = match base {
            Some(_) => tr!("El anuncio HTH de la posicion {} ahora usa la base {}", indice + 1, nombre_de_archivo(&nuevo)),
            None => tr!("El anuncio HTH de la posicion {} quedo sin base: la voz sola, sin mezcla", indice + 1),
        };
        if base.is_some() {
            self.ultima_base_hth = Some(nuevo);
        }
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Aviso para el editor al guardar una base: si su ventana es mas corta
    /// que la voz.
    pub(super) fn aviso_de_ventana_de_la_base(&self, pista: &Pista) -> Option<String> {
        if !self.es_base_del_anuncio_hth(pista) {
            return None;
        }
        let ventana = self.lista.puntos_de(pista).overlay().map(|ventana| ventana.fin() - ventana.inicio());
        aviso_de_ventana(ventana, self.duracion_de_la_voz_hth.unwrap_or(DURACION_TIPICA_DE_LA_VOZ))
    }

    /// Antes de que suene el anuncio: lo arma con la hora de este momento.
    /// Si no se puede, devuelve false y el anuncio no debe sonar (diria una
    /// hora vieja).
    pub(super) fn preparar_anuncio_hth(&mut self) -> bool {
        self.iniciar_clima();
        match self.armar_anuncio_hth() {
            Ok(resumen) => {
                self.mostrar_estado(&tr!("Anuncio HTH: {resumen}", resumen = resumen));
                true
            }
            Err(error) => {
                log::warn!("No se pudo armar el anuncio HTH: {error}");
                self.mostrar_estado(&tr!("Se salteo el anuncio HTH: {error}", error = error));
                false
            }
        }
    }

    /// La proxima pista de la lista despues de la actual que no sea un
    /// anuncio HTH (para seguir cuando no se pudo armar).
    pub(super) fn proxima_sin_anuncio_hth(&self) -> Option<usize> {
        let sin_anuncio = |pista: &Pista| archivo_existe(pista) && !es_anuncio_hth(pista.archivo());
        self.lista.proxima_disponible(self.lista.indice_actual(), sin_anuncio).0
    }

    /// Al recuperar o cargar una lista con el anuncio: si falta su archivo
    /// (por ejemplo, se borro la carpeta de datos), lo arma para que la pista
    /// no se pierda; y lee su duracion para mostrarla.
    pub(super) fn preparar_lista_con_anuncio_hth(&mut self, rutas: &[PathBuf]) {
        if !rutas.iter().any(|ruta| es_anuncio_hth(ruta)) {
            return;
        }
        self.iniciar_clima();
        let archivo = pista_del_anuncio_hth().archivo();
        if !archivo.is_file() {
            if let Err(error) = self.armar_anuncio_hth() {
                log::warn!("La lista tiene anuncios HTH pero no se pudieron armar: {error}");
            }
        } else if self.duracion_del_anuncio_hth.is_none() {
            self.duracion_del_anuncio_hth = Decodificador::sondear_duracion(archivo).ok().flatten();
        }
    }
}

/// Si la ventana de overlay de una base no alcanza para la voz.
fn aviso_de_ventana(ventana: Option<f64>, voz: f64) -> Option<String> {
    match ventana {
        None => Some(tr!("la base no tiene ventana de overlay: la voz arranca con la base").into()),
        Some(segundos) if segundos + 0.05 < voz => Some(tr!("la ventana de overlay dura {segundos:.1} s y la voz {voz:.1} s: la voz sigue despues de la ventana", segundos = segundos, voz = voz)),
        Some(_) => None,
    }
}

/// Boton HTH: inserta el anuncio; si todavia no hay locuciones, primero pide
/// la carpeta (el dialogo se muestra fuera del controlador, para no frenar la
/// reproduccion).
pub(super) fn insertar_anuncio_hth_con_dialogo(ventana: &slint::Weak<VentanaPrincipal>) {
    let mut faltan = false;
    con_controlador(|c| faltan = c.locuciones().is_none());
    if faltan && !elegir_carpeta_de_locuciones_con_dialogo(ventana) {
        return;
    }
    con_controlador(|c| {
        if let Err(error) = c.insertar_anuncio_hth(None) {
            c.mostrar_estado(&error);
        }
    });
}

/// Pide la carpeta de las locuciones y la guarda si sirve. Devuelve si quedo
/// elegida una carpeta valida.
pub(super) fn elegir_carpeta_de_locuciones_con_dialogo(ventana: &slint::Weak<VentanaPrincipal>) -> bool {
    let mut dialogo = rfd::FileDialog::new()
        .set_title(&tr!("Carpeta de las locuciones del anuncio HTH (con hora, minutos, temperatura y humedad)"));
    if let Some(principal) = ventana.upgrade() {
        dialogo = dialogo.set_parent(&principal.window().window_handle());
    }
    let Some(carpeta) = dialogo.pick_folder() else {
        return false;
    };
    let mut resultado = Err(String::new());
    con_controlador(|c| resultado = c.elegir_carpeta_de_locuciones(&carpeta));
    match resultado {
        Ok(mensaje) => {
            con_controlador(|c| c.mostrar_estado(&mensaje));
            true
        }
        Err(error) => {
            let mut aviso = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title(&tr!("Locuciones del anuncio HTH"))
                .set_description(&error)
                .set_buttons(rfd::MessageButtons::Ok);
            if let Some(principal) = ventana.upgrade() {
                aviso = aviso.set_parent(&principal.window().window_handle());
            }
            aviso.show();
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_clima_se_muestra_con_su_antiguedad_o_sin_datos() {
        let clima = crate::servicio::Clima { temperatura: 17.44, humedad: 76.2 };
        let lectura = |sin_conexion: bool, ultima: Option<(crate::servicio::Clima, Duration)>| LecturaDeClima {
            iniciado: true,
            sin_conexion,
            ultima,
        };
        let minutos = |cantidad: u64| Duration::from_secs(cantidad * 60);

        let (texto, sin_datos) = resumen_del_clima("Buenos Aires", &lectura(false, Some((clima, minutos(3)))));
        assert_eq!(texto, "Buenos Aires: 17,4 °C, humedad 76 %
Ultima lectura hace 3 min");
        assert!(!sin_datos);

        // Fallo la ultima consulta pero el dato todavia sirve.
        let (texto, sin_datos) = resumen_del_clima("Buenos Aires", &lectura(true, Some((clima, minutos(25)))));
        assert!(texto.ends_with("hace 25 min (sin conexion: reintentando)") && !sin_datos);

        // Pasada una hora, el dato ya no se anuncia: sin datos.
        let (texto, sin_datos) = resumen_del_clima("Buenos Aires", &lectura(true, Some((clima, minutos(65)))));
        assert_eq!(texto, "Buenos Aires: sin datos
Sin conexion; ultima lectura hace 1 h 5 min");
        assert!(sin_datos);

        assert_eq!(resumen_del_clima("Cordoba", &lectura(true, None)).0, "Cordoba: sin datos
Sin conexion con el servicio del clima");
        assert_eq!(resumen_del_clima("Cordoba", &lectura(false, None)).0, "Cordoba: consultando el clima...");
        let sin_iniciar = LecturaDeClima { iniciado: false, sin_conexion: false, ultima: None };
        assert!(resumen_del_clima("Cordoba", &sin_iniciar).0.ends_with("Se consulta al usar el anuncio HTH"));
        assert_eq!(hace_cuanto(Duration::from_secs(2)), "recien");
        assert_eq!(hace_cuanto(Duration::from_secs(40)), "hace 40 s");
        assert_eq!(hace_cuanto(Duration::from_secs(7200)), "hace 2 h");
    }

    #[test]
    fn el_reloj_muestra_el_clima_en_una_linea() {
        let clima = crate::servicio::Clima { temperatura: 17.44, humedad: 76.2 };
        let con_dato = LecturaDeClima { iniciado: true, sin_conexion: false, ultima: Some((clima, Duration::from_secs(60))) };
        assert_eq!(clima_para_el_reloj("Rosario", &con_dato), "17,4 °C · humedad 76 % · Rosario");
        let vencido = LecturaDeClima { iniciado: true, sin_conexion: true, ultima: Some((clima, Duration::from_secs(7200))) };
        assert_eq!(clima_para_el_reloj("Rosario", &vencido), "Rosario: sin datos del clima");
        let consultando = LecturaDeClima { iniciado: true, sin_conexion: false, ultima: None };
        assert_eq!(clima_para_el_reloj("Rosario", &consultando), "Rosario: consultando el clima...");
        assert_eq!(clima_para_el_reloj("Rosario", &LecturaDeClima { iniciado: false, sin_conexion: false, ultima: None }), "");
    }

    #[test]
    fn las_bases_se_reconocen_por_su_carpeta() {
        let bases = PathBuf::from("D:/locuciones/bases");
        assert!(es_base_hth(&bases.join("Base 1.mp3"), Some(&bases)));
        assert!(!es_base_hth(&PathBuf::from("D:/musica/bases/Base 1.mp3"), Some(&bases)));
        assert!(!es_base_hth(&bases.join("Base 1.mp3"), None));
        assert!(podria_ser_base(&bases.join("Base 1.mp3")));
        assert!(!podria_ser_base(&PathBuf::from("D:/musica/tema.mp3")));
    }

    #[test]
    fn avisa_si_la_ventana_no_alcanza_para_la_voz() {
        assert_eq!(aviso_de_ventana(Some(12.0), 9.0), None);
        assert!(aviso_de_ventana(Some(6.0), 9.0).unwrap().contains("6.0 s"));
        assert!(aviso_de_ventana(None, 9.0).unwrap().contains("no tiene ventana"));
    }

    #[test]
    fn el_anuncio_se_reconoce_por_su_ruta() {
        let anuncio = pista_del_anuncio_hth().archivo().to_path_buf();
        assert!(es_anuncio_hth(&anuncio));
        assert!(!es_anuncio_hth(&PathBuf::from("D:/musica").join(NOMBRE_DEL_ARCHIVO)));
        assert!(!es_anuncio_hth(&anuncio.with_file_name("otro.wav")));
    }

    #[test]
    fn el_anuncio_de_otra_carpeta_de_datos_pasa_a_la_de_este_equipo() {
        let anuncio = pista_del_anuncio_hth().archivo().to_path_buf();
        let de_windows = PathBuf::from(format!(r"C:\Users\otro\.mcp-raptor-player\servicios\{NOMBRE_DEL_ARCHIVO}"));
        let de_linux = PathBuf::from(format!("/home/radio/.mcp-raptor-player/servicios/{NOMBRE_DEL_ARCHIVO}"));
        assert_eq!(anuncio_hth_en_esta_carpeta(&de_windows), Some(anuncio.clone()));
        assert_eq!(anuncio_hth_en_esta_carpeta(&de_linux), Some(anuncio));
        assert_eq!(anuncio_hth_en_esta_carpeta(&PathBuf::from("D:/musica").join(NOMBRE_DEL_ARCHIVO)), None);
        assert_eq!(anuncio_hth_en_esta_carpeta(&PathBuf::from("D:/servicios/otro.wav")), None);
    }
}
