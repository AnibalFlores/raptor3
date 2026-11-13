//! Ubicacion de las ventanas que se abren desde los menus (buscador,
//! mantenimiento, pauta, ordenes, Acerca de): aparecen centradas sobre la
//! ventana principal y, si quedaron en otra pantalla (o en una que ya no
//! esta conectada), vuelven a ella.

use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, PhysicalPosition, PhysicalSize};

use super::Controlador;
use super::editor::AreaDePantalla;
use crate::AcercaDe;

/// Indice de la pantalla que contiene ese punto, si alguna lo contiene.
pub(super) fn pantalla_con(x: i32, y: i32, pantallas: &[AreaDePantalla]) -> Option<usize> {
    pantallas.iter().position(|pantalla| {
        let (x, y) = (i64::from(x), i64::from(y));
        let (izquierda, arriba) = (i64::from(pantalla.x), i64::from(pantalla.y));
        x >= izquierda && x < izquierda + i64::from(pantalla.ancho) && y >= arriba && y < arriba + i64::from(pantalla.alto)
    })
}

/// Centro de un rectangulo de la pantalla.
fn centro(posicion: PhysicalPosition, tamano: PhysicalSize) -> (i32, i32) {
    (posicion.x + (tamano.width / 2) as i32, posicion.y + (tamano.height / 2) as i32)
}

/// Posicion para centrar `tamano` sobre el area de la principal (sin salirse
/// por arriba ni por la izquierda de ella).
pub(super) fn posicion_centrada(origen: PhysicalPosition, area: PhysicalSize, tamano: PhysicalSize) -> PhysicalPosition {
    let centrar = |desde: i32, disponible: u32, ocupado: u32| desde + ((disponible as i32 - ocupado as i32) / 2).max(0);
    PhysicalPosition::new(centrar(origen.x, area.width, tamano.width), centrar(origen.y, area.height, tamano.height))
}

impl Controlador {
    /// Centra la ventana sobre la principal. Si es mas grande que ella, la
    /// achica a su tamano. Sin tamano conocido todavia (nunca se mostro), no
    /// hace nada.
    pub(super) fn centrar_sobre_la_principal(&self, ventana: &slint::Window) {
        let Some(principal) = self.ventana.upgrade() else { return };
        let principal = principal.window();
        let (origen, area) = (principal.position(), principal.size());
        let tamano = ventana.size();
        if tamano.width == 0 || tamano.height == 0 {
            return;
        }
        let ajustado = PhysicalSize::new(tamano.width.min(area.width.max(1)), tamano.height.min(area.height.max(1)));
        if ajustado != tamano {
            ventana.set_size(ajustado);
        }
        ventana.set_position(posicion_centrada(origen, area, ajustado));
    }

    /// Si la ventana esta en la misma pantalla que la principal (su centro).
    fn en_la_pantalla_de_la_principal(&self, ventana: &slint::Window) -> bool {
        let Some(principal) = self.ventana.upgrade() else { return true };
        let pantallas = self.pantallas();
        let (px, py) = centro(principal.window().position(), principal.window().size());
        let (vx, vy) = centro(ventana.position(), ventana.size());
        match (pantalla_con(px, py, &pantallas), pantalla_con(vx, vy, &pantallas)) {
            (Some(de_la_principal), Some(de_la_ventana)) => de_la_principal == de_la_ventana,
            // Fuera de todas las pantallas: hay que traerla.
            (Some(_), None) => false,
            _ => true,
        }
    }

    /// Muestra una ventana abierta desde un menu: cerrada, aparece centrada
    /// sobre la principal; abierta en otra pantalla (o fuera de las
    /// conectadas), vuelve a ella; abierta en la misma, solo pasa al frente.
    pub(super) fn mostrar_sobre_la_principal(
        &self,
        ventana: &slint::Window,
        mostrar: impl FnOnce() -> Result<(), slint::PlatformError>,
    ) -> Result<(), String> {
        let estaba_abierta = ventana.is_visible();
        let reubicar = !estaba_abierta || !self.en_la_pantalla_de_la_principal(ventana);
        if reubicar {
            self.centrar_sobre_la_principal(ventana);
        }
        mostrar().map_err(|error| tr!("No se pudo abrir la ventana: {error}", error = error))?;
        if reubicar {
            // Algunos sistemas solo respetan la posicion (y recien conocen el
            // tamano) con la ventana ya visible.
            self.centrar_sobre_la_principal(ventana);
        }
        if estaba_abierta {
            // Ya abierta, mostrarla no la trae al frente: se le da el foco.
            ventana.with_winit_window(|ventana| ventana.focus_window());
        }
        Ok(())
    }

    /// Menu Ayuda: Acerca de, centrado sobre la principal.
    pub(super) fn abrir_acerca_de(&mut self, acerca: &AcercaDe) {
        self.acerca = Some(acerca.as_weak());
        if let Err(error) = self.mostrar_sobre_la_principal(acerca.window(), || acerca.show()) {
            self.mostrar_estado(&error);
        }
    }

    /// Menu Ventana: trae sobre la ventana principal todas las ventanas
    /// abiertas (el editor, el buscador, el mantenimiento, la pauta, las
    /// ordenes, el selector de avisos, los anunciantes y Acerca de).
    pub(super) fn traer_ventanas(&mut self) {
        let mut traidas = 0;
        if self.editor.window().is_visible() {
            self.traer_editor();
            traidas += 1;
        }
        let acerca = self.acerca.as_ref().and_then(slint::Weak::upgrade);
        let ventanas: Vec<&slint::Window> = [
            Some(self.buscador.ventana()),
            Some(self.mantenimiento.window()),
            Some(self.pauta.ventana()),
            Some(self.ventana_de_ordenes.ventana()),
            Some(self.avisos_del_catalogo.ventana()),
            Some(self.anunciantes_de_la_ventana.ventana()),
            Some(self.selector_de_localidad.ventana()),
            Some(self.actualizacion.ventana()),
            acerca.as_ref().map(|acerca| acerca.window()),
        ]
        .into_iter()
        .flatten()
        .filter(|ventana| ventana.is_visible())
        .collect();
        for ventana in &ventanas {
            self.centrar_sobre_la_principal(ventana);
        }
        traidas += ventanas.len();
        self.mostrar_estado(&if traidas == 0 {
            tr!("No hay otras ventanas abiertas.")
        } else {
            tr!("{traidas} ventana(s) centrada(s) sobre la ventana principal.", traidas = traidas)
        });
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn se_reconoce_la_pantalla_de_cada_punto() {
        let pantallas = [
            AreaDePantalla { x: 0, y: 0, ancho: 1920, alto: 1080 },
            AreaDePantalla { x: 1920, y: 0, ancho: 2560, alto: 1440 },
            AreaDePantalla { x: -1280, y: 0, ancho: 1280, alto: 1024 },
        ];
        assert_eq!(pantalla_con(960, 540, &pantallas), Some(0));
        assert_eq!(pantalla_con(2500, 700, &pantallas), Some(1));
        assert_eq!(pantalla_con(-10, 10, &pantallas), Some(2));
        assert_eq!(pantalla_con(500, 2000, &pantallas), None, "fuera de las pantallas");
        let centrada = posicion_centrada(PhysicalPosition::new(1920, 0), PhysicalSize::new(2560, 1400), PhysicalSize::new(1000, 600));
        assert_eq!(centrada, PhysicalPosition::new(1920 + 780, 400));
        // Mas grande que la principal: queda en su esquina.
        let grande = posicion_centrada(PhysicalPosition::new(0, 0), PhysicalSize::new(800, 600), PhysicalSize::new(1000, 700));
        assert_eq!(grande, PhysicalPosition::new(0, 0));
    }
}
