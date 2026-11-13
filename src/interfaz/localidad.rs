//! Localidad del clima del anuncio HTH: se busca por nombre en el
//! geocodificador gratuito de Open-Meteo (en un hilo, sin trabar la interfaz)
//! y se elige de la lista, que muestra la region para distinguir homonimas.

use std::rc::Rc;

use slint::{CloseRequestResponse, ComponentHandle, ModelRc, VecModel};

use super::{Controlador, con_controlador};
use crate::servicio::{LocalidadEncontrada, buscar_localidades};
use crate::{LocalidadEncontrada as FilaDeLocalidad, SelectorDeLocalidad};

/// La ventana del selector y las localidades que muestra.
pub(super) struct SelectorDeLocalidadDelClima {
    ventana: SelectorDeLocalidad,
    modelo: Rc<VecModel<FilaDeLocalidad>>,
    encontradas: Vec<LocalidadEncontrada>,
    // Sube con cada busqueda: la respuesta de una vieja se descarta.
    busqueda: u64,
}

impl SelectorDeLocalidadDelClima {
    pub(super) fn nuevo() -> Result<Self, String> {
        let ventana =
            SelectorDeLocalidad::new().map_err(|error| tr!("No se pudo crear el selector de localidad: {error}", error = error))?;
        let modelo = Rc::new(VecModel::default());
        ventana.set_localidades(ModelRc::from(modelo.clone()));
        ventana.on_buscar(|| con_controlador(Controlador::buscar_localidades_del_clima));
        ventana.on_usar(|indice| con_controlador(|c| c.usar_localidad_encontrada(indice)));
        ventana.on_cerrar(|| con_controlador(|c| c.cerrar_selector_de_localidad()));
        ventana.window().on_close_requested(|| CloseRequestResponse::HideWindow);
        Ok(Self { ventana, modelo, encontradas: Vec::new(), busqueda: 0 })
    }

    pub(super) fn ventana(&self) -> &slint::Window {
        self.ventana.window()
    }
}

/// Fila de la lista para una localidad encontrada.
fn fila(encontrada: &LocalidadEncontrada) -> FilaDeLocalidad {
    let localidad = &encontrada.localidad;
    FilaDeLocalidad {
        nombre: localidad.nombre.clone().into(),
        region: encontrada.region.clone().into(),
        coordenadas: format!("{:.2}, {:.2}", localidad.latitud, localidad.longitud).into(),
    }
}

impl Controlador {
    /// Menu del boton HTH: abre (o trae al frente) el selector de localidad.
    pub(super) fn abrir_selector_de_localidad(&mut self) {
        self.refrescar_localidad_en_el_selector();
        let ventana = self.selector_de_localidad.ventana.clone_strong();
        if let Err(error) = self.mostrar_sobre_la_principal(ventana.window(), || ventana.show()) {
            self.mostrar_estado(&error);
            return;
        }
        ventana.invoke_enfocar();
    }

    /// Muestra en el selector la localidad en uso y su clima.
    fn refrescar_localidad_en_el_selector(&self) {
        let ventana = &self.selector_de_localidad.ventana;
        ventana.set_actual(self.ventana.upgrade().map(|principal| principal.get_clima_hth()).unwrap_or_default().replace('\n', " · ").into());
    }

    fn cerrar_selector_de_localidad(&mut self) {
        let _ = self.selector_de_localidad.ventana.hide();
    }

    /// Busca en un hilo las localidades con el nombre escrito.
    fn buscar_localidades_del_clima(&mut self) {
        let selector = &mut self.selector_de_localidad;
        let nombre = selector.ventana.get_texto().trim().to_string();
        if nombre.is_empty() {
            return;
        }
        selector.busqueda += 1;
        let busqueda = selector.busqueda;
        selector.ventana.set_buscando(true);
        selector.ventana.set_resumen(tr!("Buscando «{nombre}»...", nombre = nombre).into());
        let lanzado = std::thread::Builder::new().name("busqueda-de-localidad".into()).spawn(move || {
            let resultado = buscar_localidades(&nombre);
            let _ = slint::invoke_from_event_loop(move || {
                con_controlador(|c| c.mostrar_localidades_encontradas(busqueda, &nombre, resultado));
            });
        });
        if let Err(error) = lanzado {
            self.selector_de_localidad.ventana.set_buscando(false);
            self.selector_de_localidad.ventana.set_resumen(tr!("No se pudo buscar: {error}", error = error).into());
        }
    }

    fn mostrar_localidades_encontradas(
        &mut self,
        busqueda: u64,
        nombre: &str,
        resultado: Result<Vec<LocalidadEncontrada>, String>,
    ) {
        let selector = &mut self.selector_de_localidad;
        if busqueda != selector.busqueda {
            return;
        }
        selector.ventana.set_buscando(false);
        let (encontradas, resumen) = match resultado {
            Ok(encontradas) if encontradas.is_empty() => {
                (encontradas, tr!("No se encontro «{nombre}». Pruebe con otro nombre o sin abreviaturas.", nombre = nombre))
            }
            Ok(encontradas) => {
                let resumen = tr!("{} localidad(es) para «{nombre}»; elija la suya por la region.", encontradas.len(), nombre = nombre);
                (encontradas, resumen)
            }
            Err(error) => (Vec::new(), tr!("{error}. Revise la conexion a internet.", error = error)),
        };
        selector.modelo.set_vec(encontradas.iter().map(fila).collect::<Vec<_>>());
        // Con una sola coincidencia queda elegida.
        selector.ventana.set_elegida(if encontradas.len() == 1 { 0 } else { -1 });
        selector.encontradas = encontradas;
        selector.ventana.set_resumen(resumen.into());
    }

    /// Usa la localidad de esa fila para el clima del anuncio y cierra.
    fn usar_localidad_encontrada(&mut self, indice: i32) {
        let Some(encontrada) = usize::try_from(indice).ok().and_then(|indice| self.selector_de_localidad.encontradas.get(indice))
        else {
            return;
        };
        let localidad = encontrada.localidad.clone();
        let region = encontrada.region.clone();
        self.cambiar_localidad_del_clima(localidad.clone());
        let _ = self.selector_de_localidad.ventana.hide();
        let donde = if region.is_empty() { localidad.nombre } else { format!("{} ({region})", localidad.nombre) };
        self.mostrar_estado(&tr!("Clima del anuncio HTH: {donde}. Consultando...", donde = donde));
    }
}
