//! Marcadores de bloque fijo en la lista: un inicio y un fin entre los que
//! Mezclar y el orden por BPM no cambian nada.
//!
//! Cada marcador es una fila de la lista que apunta a un archivo chico de la
//! carpeta de datos (asi la lista lo guarda y lo recupera como cualquier
//! otra pista). No suena: la reproduccion y las mezclas lo saltean, sin
//! avisar, y no se analiza ni entra al catalogo.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::Controlador;
use crate::dominio::{MarcadorDeBloque, Pista, bloque_de, posiciones_fijas};
use crate::persistencia::carpeta_del_usuario;

/// Como se muestran los marcadores en la lista.
pub(super) const TIPO_DEL_MARCADOR: &str = "Bloque fijo";
pub(super) fn ayuda_del_marcador() -> String {
    tr!("Mezclar y BPM no cambian lo que queda dentro")
}

const CONTENIDO: &str = "Marcador de bloque fijo de MCP Raptor Player: Mezclar y BPM no cambian lo que queda entre el inicio y el fin.\n";

pub(super) fn titulo_del_marcador(marcador: MarcadorDeBloque) -> String {
    match marcador {
        MarcadorDeBloque::Inicio => tr!("Inicio de bloque fijo"),
        MarcadorDeBloque::Fin => tr!("Fin de bloque fijo"),
    }
}

/// La pista de cada marcador: siempre el mismo archivo, en la carpeta de datos.
fn pista_del_marcador(marcador: MarcadorDeBloque) -> &'static Pista {
    static INICIO: OnceLock<Pista> = OnceLock::new();
    static FIN: OnceLock<Pista> = OnceLock::new();
    let (celda, nombre) = match marcador {
        MarcadorDeBloque::Inicio => (&INICIO, "Inicio de bloque fijo.marca"),
        MarcadorDeBloque::Fin => (&FIN, "Fin de bloque fijo.marca"),
    };
    celda.get_or_init(|| Pista::new(carpeta_del_usuario().join("marcadores").join(nombre)))
}

/// El marcador al que apunta el archivo, si es uno.
pub(super) fn marcador_de(archivo: &Path) -> Option<MarcadorDeBloque> {
    // Descarte rapido por la extension: se pregunta por cada fila.
    if archivo.extension().is_none_or(|extension| extension != "marca") {
        return None;
    }
    let pista = Pista::new(archivo);
    [MarcadorDeBloque::Inicio, MarcadorDeBloque::Fin].into_iter().find(|marcador| *pista_del_marcador(*marcador) == pista)
}

pub(super) fn es_marcador(archivo: &Path) -> bool {
    marcador_de(archivo).is_some()
}

/// Crea el archivo del marcador si falta (la lista solo recupera pistas
/// cuyo archivo existe).
fn asegurar_archivo(marcador: MarcadorDeBloque) -> Result<PathBuf, String> {
    let archivo = pista_del_marcador(marcador).archivo().to_path_buf();
    if !archivo.is_file() {
        if let Some(carpeta) = archivo.parent() {
            std::fs::create_dir_all(carpeta).map_err(|error| tr!("No se pudo crear {}: {error}", carpeta.display(), error = error))?;
        }
        std::fs::write(&archivo, CONTENIDO).map_err(|error| tr!("No se pudo crear {}: {error}", archivo.display(), error = error))?;
    }
    Ok(archivo)
}

impl Controlador {
    /// Inserta un marcador en la posicion indicada (desde 0) o, con None, en
    /// el punto de insercion (despues de la seleccionada).
    pub fn insertar_marcador(&mut self, marcador: MarcadorDeBloque, posicion: Option<usize>) -> Result<String, String> {
        let archivo = asegurar_archivo(marcador)?;
        let posicion = posicion.unwrap_or_else(|| self.punto_de_insercion()).min(self.lista.tamano());
        self.insertar_pistas(vec![archivo], Some(posicion), true);
        let mensaje = tr!("{} en la posicion {}: Mezclar y BPM no cambian lo que queda entre el inicio y el fin", titulo_del_marcador(marcador), posicion + 1);
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Encierra entre un inicio y un fin las pistas de `desde` a `hasta`
    /// (desde 0, inclusive).
    pub fn fijar_bloque(&mut self, desde: usize, hasta: usize) -> Result<String, String> {
        let tamano = self.lista.tamano();
        if desde > hasta || hasta >= tamano {
            return Err(tr!("Indique un tramo de la lista: desde y hasta entre 1 y {tamano}, desde no mayor que hasta.", tamano = tamano));
        }
        let (inicio, fin) = (asegurar_archivo(MarcadorDeBloque::Inicio)?, asegurar_archivo(MarcadorDeBloque::Fin)?);
        // Primero el fin, para que no se corra la posicion del inicio.
        self.insertar_pistas(vec![fin], Some(hasta + 1), true);
        self.insertar_pistas(vec![inicio], Some(desde), true);
        let mensaje = tr!("Bloque fijo de {} pista(s), de la posicion {} a la {}: Mezclar y BPM no lo cambian", hasta - desde + 1, desde + 2, hasta + 2);
        self.mostrar_estado(&mensaje);
        Ok(mensaje)
    }

    /// Encierra la seleccion (de la primera a la ultima seleccionada) en un
    /// bloque fijo.
    pub fn fijar_seleccion_como_bloque(&mut self) -> Result<String, String> {
        let (Some(&desde), Some(&hasta)) = (self.seleccion.first(), self.seleccion.last()) else {
            return Err(tr!("Seleccione las pistas que forman el bloque fijo.").into());
        };
        self.fijar_bloque(desde, hasta)
    }

    /// Al recuperar o cargar una lista con marcadores: crea sus archivos si
    /// faltan, para que no se pierdan.
    pub(super) fn preparar_lista_con_marcadores(&self, rutas: &[PathBuf]) {
        for marcador in [MarcadorDeBloque::Inicio, MarcadorDeBloque::Fin] {
            if rutas.iter().any(|ruta| marcador_de(ruta) == Some(marcador)) {
                if let Err(error) = asegurar_archivo(marcador) {
                    log::warn!("{error}");
                }
            }
        }
    }

    pub(super) fn marcadores_de_la_lista(&self) -> Vec<Option<MarcadorDeBloque>> {
        self.lista.pistas().iter().map(|pista| marcador_de(pista.archivo())).collect()
    }

    /// Para cada posicion de la lista, si esta en un bloque fijo.
    pub(super) fn posiciones_en_bloques_fijos(&self) -> Vec<bool> {
        posiciones_fijas(&self.marcadores_de_la_lista())
    }

    /// Si la fila es un marcador, selecciona su bloque completo (con los dos
    /// marcadores) y devuelve true; si no, no hace nada.
    pub(super) fn seleccionar_bloque(&mut self, indice: usize) -> bool {
        if self.lista.pista_en(indice).is_none_or(|pista| !es_marcador(pista.archivo())) {
            return false;
        }
        let Some((desde, hasta)) = bloque_de(&self.marcadores_de_la_lista(), indice) else {
            return false;
        };
        self.seleccion = (desde..=hasta).collect();
        self.ancla_de_seleccion = Some(indice);
        self.refrescar_filas();
        let temas = self.temas_entre(desde, hasta);
        self.mostrar_estado(&format!(
            "Bloque fijo seleccionado: {temas} tema(s), de la posicion {} a la {}. Quitar saca primero los marcadores; \
             pulsandolo otra vez se quitan los temas",
            desde + 1,
            hasta + 1
        ));
        true
    }

    /// Si encolar esa fila como siguiente la saca de un bloque fijo, el aviso
    /// para confirmarlo. No la saca si la que suena esta en el mismo bloque:
    /// entonces queda dentro, justo despues de ella.
    pub fn aviso_de_salida_de_bloque(&self, indice: usize) -> Option<String> {
        let pista = self.lista.pista_en(indice).filter(|pista| !es_marcador(pista.archivo()))?;
        // La que suena no se encola: lo informa encolar_como_siguiente.
        if self.lista.indice_actual() == Some(indice) {
            return None;
        }
        let marcadores = self.marcadores_de_la_lista();
        let (desde, hasta) = bloque_de(&marcadores, indice)?;
        let actual = self.lista.indice_actual();
        let suena_en_el_bloque = actual.is_some_and(|actual| {
            marcadores.get(actual).is_some_and(Option::is_none) && bloque_de(&marcadores, actual) == Some((desde, hasta))
        });
        if suena_en_el_bloque {
            return None;
        }
        let quedan = self.temas_entre(desde, hasta) - 1;
        Some(format!(
            "{} esta dentro del bloque fijo de la posicion {} a la {}.\n\n\
             Al encolarlo como siguiente sale del bloque: Mezclar y BPM ya no lo dejan fijo y el bloque queda con \
             {quedan} tema(s).\n\n¿Encolarlo igual?",
            super::etiqueta(&self.lista, pista),
            desde + 1,
            hasta + 1
        ))
    }

    /// Cuantos temas (filas que no son marcadores) hay entre esas posiciones.
    fn temas_entre(&self, desde: usize, hasta: usize) -> usize {
        (desde..=hasta)
            .filter(|&posicion| self.lista.pista_en(posicion).is_some_and(|pista| !es_marcador(pista.archivo())))
            .count()
    }

    /// Si entre las posiciones a quitar hay marcadores y temas, quita solo
    /// los marcadores (deshace el bloque) y deja seleccionados los temas,
    /// para que un segundo Quitar los saque. Devuelve true si lo hizo.
    pub(super) fn quitar_solo_los_marcadores(&mut self, posiciones: &[usize]) -> bool {
        let es_marca = |posicion: &usize| self.lista.pista_en(*posicion).is_some_and(|pista| es_marcador(pista.archivo()));
        let (marcadores, temas): (Vec<usize>, Vec<usize>) = posiciones.iter().copied().partition(es_marca);
        if marcadores.is_empty() || temas.is_empty() {
            return false;
        }
        self.quitar_posiciones(&marcadores);
        // Cada tema baja tantos lugares como marcadores habia antes que el.
        let corrida = |posicion: usize| posicion - marcadores.iter().filter(|&&marcador| marcador < posicion).count();
        self.seleccion = temas.iter().copied().map(corrida).collect();
        self.ancla_de_seleccion = self.seleccion.last().copied();
        self.refrescar_filas();
        self.mostrar_estado(&format!(
            "Se quitaron {} marcador(es) de bloque fijo; los {} tema(s) siguen en la lista y seleccionados: \
             pulse Quitar otra vez para quitarlos",
            marcadores.len(),
            temas.len()
        ));
        true
    }

    /// Las posiciones que se pueden reordenar (las que no estan en un bloque
    /// fijo) y cuantos temas quedaron afuera (sin contar los marcadores).
    pub(super) fn sin_bloques_fijos(&self, posiciones: &[usize]) -> (Vec<usize>, usize) {
        let fijas = self.posiciones_en_bloques_fijos();
        let fija = |posicion: usize| fijas.get(posicion).copied().unwrap_or(false);
        let libres: Vec<usize> = posiciones.iter().copied().filter(|&posicion| !fija(posicion)).collect();
        let temas_fijos = posiciones
            .iter()
            .filter(|&&posicion| fija(posicion))
            .filter(|&&posicion| self.lista.pista_en(posicion).is_some_and(|pista| !es_marcador(pista.archivo())))
            .count();
        (libres, temas_fijos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_marcadores_se_reconocen_por_su_archivo() {
        let inicio = pista_del_marcador(MarcadorDeBloque::Inicio).archivo().to_path_buf();
        let fin = pista_del_marcador(MarcadorDeBloque::Fin).archivo().to_path_buf();
        assert_eq!(marcador_de(&inicio), Some(MarcadorDeBloque::Inicio));
        assert_eq!(marcador_de(&fin), Some(MarcadorDeBloque::Fin));
        assert_eq!(marcador_de(&PathBuf::from("D:/musica/Inicio de bloque fijo.marca")), None);
        assert_eq!(marcador_de(&PathBuf::from("D:/musica/tema.mp3")), None);
    }
}
