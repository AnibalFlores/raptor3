//! Modo de operacion AL AIRE / EDITOR: al aire, los comandos que rompen la
//! secuencia que sale (adelantar, volver, saltar a otra pista, reordenar,
//! vaciar o reemplazar la lista, quitar o mover varias a la vez) se rechazan
//! con un aviso, vengan de la ventana o del MCP.

use super::Controlador;
use crate::dominio::{ModoDeOperacion, OperacionSobreLaSecuencia};

/// Por que no se admite la operacion al aire.
fn motivo(operacion: OperacionSobreLaSecuencia) -> String {
    use OperacionSobreLaSecuencia::*;
    let que = match operacion {
        Adelantar => tr!("Al aire no se adelanta ni se retrocede dentro del tema"),
        Anterior => tr!("Al aire no se vuelve a la pista anterior"),
        SaltarA => tr!("Al aire no se salta a otra pista mientras suena una: use Siguiente o Encolar como siguiente"),
        SiguienteMarcada => tr!("Al aire Siguiente sigue el orden de la lista"),
        Reordenar => tr!("Al aire no se reordena la lista (mezclar u ordenar por BPM)"),
        Limpiar => tr!("Al aire no se vacía la lista"),
        CargarLista => tr!("Al aire no se reemplaza la lista"),
        Quitar(_) => tr!("Al aire los temas se quitan de a uno"),
        Mover(_) => tr!("Al aire los temas se mueven de a uno"),
        CambiarLocalidad => tr!("Al aire no se cambia la localidad del HTH (de ella salen la hora y los horarios de la lista)"),
    };
    tr!("{que} (pase a modo Editor para hacerlo).", que = que)
}

impl Controlador {
    pub fn modo_de_operacion(&self) -> ModoDeOperacion {
        self.repositorio_de_preferencias.modo_de_operacion()
    }

    /// Cambia entre AL AIRE y EDITOR y lo recuerda para la proxima vez.
    pub fn cambiar_modo_de_operacion(&mut self, modo: ModoDeOperacion) -> String {
        self.repositorio_de_preferencias.guardar_modo_de_operacion(modo);
        self.con_ventana(|ventana| ventana.set_al_aire(modo.al_aire()));
        // Al aire no se puede adelantar: se descarta un arrastre a medias, y
        // tampoco se cambia la localidad: se cierra su selector.
        if modo.al_aire() {
            self.busqueda_pendiente = None;
            self.cerrar_selector_de_localidad();
        }
        let mensaje = if modo.al_aire() {
            tr!("Modo AL AIRE: no se adelanta, no se vuelve ni se saltea; Detener y Siguiente mezclan con el fader, y los temas se agregan o quitan de a uno")
        } else {
            tr!("Modo EDITOR: todos los comandos habilitados")
        };
        self.mostrar_estado(&mensaje);
        mensaje
    }

    /// Ok si el modo admite la operacion; si no, el aviso para el usuario.
    pub fn admitida(&self, operacion: OperacionSobreLaSecuencia) -> Result<(), String> {
        if self.modo_de_operacion().admite(operacion) { Ok(()) } else { Err(motivo(operacion)) }
    }

    /// Como `admitida`, pero el aviso va a la barra de estado.
    pub(super) fn admitida_o_avisar(&mut self, operacion: OperacionSobreLaSecuencia) -> bool {
        match self.admitida(operacion) {
            Ok(()) => true,
            Err(aviso) => {
                self.mostrar_estado(&aviso);
                false
            }
        }
    }
}
