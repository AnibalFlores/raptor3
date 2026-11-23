//! Modo de operacion del reproductor: AL AIRE protege la secuencia que sale
//! al aire; EDITOR deja hacer todo.

/// Como reacciona el reproductor a los comandos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModoDeOperacion {
    /// Todo permitido (como siempre).
    #[default]
    Editor,
    /// No se adelanta ni se saltea: se puede detener y pasar a la siguiente
    /// con la mezcla del fader, y agregar o quitar temas de a uno.
    AlAire,
}

/// Comandos que en modo AL AIRE pueden romper la secuencia de reproduccion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperacionSobreLaSecuencia {
    /// Mover la posicion dentro del tema que suena.
    Adelantar,
    /// Volver a la pista anterior.
    Anterior,
    /// Hacer sonar otra pista al instante mientras suena una.
    SaltarA,
    /// Pasar con Siguiente a la pista marcada con el selector (saltea las
    /// que hay en el medio).
    SiguienteMarcada,
    /// Mezclar al azar u ordenar por BPM.
    Reordenar,
    /// Vaciar la lista.
    Limpiar,
    /// Reemplazar la lista por otra.
    CargarLista,
    /// Quitar esa cantidad de pistas de una vez.
    Quitar(usize),
    /// Mover esa cantidad de pistas de una vez.
    Mover(usize),
    /// Cambiar la localidad del HTH: cambia la hora de referencia del reloj
    /// y de los horarios de la lista.
    CambiarLocalidad,
}

impl ModoDeOperacion {
    pub fn al_aire(self) -> bool {
        self == Self::AlAire
    }

    /// Si el modo admite la operacion.
    pub fn admite(self, operacion: OperacionSobreLaSecuencia) -> bool {
        use OperacionSobreLaSecuencia::*;
        match self {
            Self::Editor => true,
            Self::AlAire => match operacion {
                Quitar(cantidad) | Mover(cantidad) => cantidad <= 1,
                Adelantar | Anterior | SaltarA | SiguienteMarcada | Reordenar | Limpiar | CargarLista | CambiarLocalidad => false,
            },
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::OperacionSobreLaSecuencia::*;
    use super::*;

    #[test]
    fn en_modo_editor_todo_se_admite() {
        for operacion in [Adelantar, Anterior, SaltarA, SiguienteMarcada, Reordenar, Limpiar, CargarLista, CambiarLocalidad, Quitar(30), Mover(5)] {
            assert!(ModoDeOperacion::Editor.admite(operacion), "{operacion:?}");
        }
        assert_eq!(ModoDeOperacion::default(), ModoDeOperacion::Editor);
    }

    #[test]
    fn al_aire_solo_se_quita_y_se_mueve_de_a_uno() {
        let al_aire = ModoDeOperacion::AlAire;
        assert!(al_aire.admite(Quitar(1)) && al_aire.admite(Mover(1)));
        assert!(!al_aire.admite(Quitar(2)) && !al_aire.admite(Mover(3)));
        for operacion in [Adelantar, Anterior, SaltarA, SiguienteMarcada, Reordenar, Limpiar, CargarLista, CambiarLocalidad] {
            assert!(!al_aire.admite(operacion), "{operacion:?}");
        }
    }
}
