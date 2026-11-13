//! Bloques fijos de la lista: lo que queda entre un marcador de inicio y uno
//! de fin no lo cambian ni Mezclar ni el orden por BPM.
//!
//! Reglas:
//! - Los marcadores no suenan y siempre quedan en su lugar.
//! - Un inicio sin fin fija hasta el final de la lista.
//! - Un fin sin inicio fija desde el fin anterior (o desde el principio).
//! - Un segundo inicio dentro de un bloque abierto no cambia nada: el bloque
//!   sigue hasta el proximo fin.

/// Marcador de bloque fijo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarcadorDeBloque {
    Inicio,
    Fin,
}

/// Tramos de la lista (desde, hasta; inclusive, desde 0) que forman cada
/// bloque fijo, con sus marcadores, en orden.
pub fn bloques_fijos(marcadores: &[Option<MarcadorDeBloque>]) -> Vec<(usize, usize)> {
    let mut bloques = Vec::new();
    // Donde empezo el bloque abierto, si hay uno.
    let mut abierto: Option<usize> = None;
    // Primera posicion despues del ultimo fin: desde ahi fija un fin huerfano.
    let mut tras_el_ultimo_fin = 0;
    for (posicion, marcador) in marcadores.iter().enumerate() {
        match marcador {
            Some(MarcadorDeBloque::Inicio) => {
                abierto.get_or_insert(posicion);
            }
            Some(MarcadorDeBloque::Fin) => {
                bloques.push((abierto.take().unwrap_or(tras_el_ultimo_fin), posicion));
                tras_el_ultimo_fin = posicion + 1;
            }
            None => {}
        }
    }
    if let Some(desde) = abierto {
        bloques.push((desde, marcadores.len() - 1));
    }
    bloques
}

/// El bloque fijo (desde, hasta; inclusive) que contiene esa posicion.
pub fn bloque_de(marcadores: &[Option<MarcadorDeBloque>], posicion: usize) -> Option<(usize, usize)> {
    bloques_fijos(marcadores).into_iter().find(|&(desde, hasta)| (desde..=hasta).contains(&posicion))
}

/// Para cada posicion de la lista (con su marcador, si es uno), si esta en
/// un bloque fijo.
pub fn posiciones_fijas(marcadores: &[Option<MarcadorDeBloque>]) -> Vec<bool> {
    let mut fijas = vec![false; marcadores.len()];
    for (desde, hasta) in bloques_fijos(marcadores) {
        fijas[desde..=hasta].iter_mut().for_each(|fija| *fija = true);
    }
    fijas
}

#[cfg(test)]
mod tests {
    use super::*;
    use MarcadorDeBloque::{Fin, Inicio};

    /// "I" inicio, "F" fin, "." tema; devuelve "x" fija, "." libre.
    fn marcadores(lista: &str) -> Vec<Option<MarcadorDeBloque>> {
        lista
            .chars()
            .map(|letra| match letra {
                'I' => Some(Inicio),
                'F' => Some(Fin),
                _ => None,
            })
            .collect()
    }

    fn fijas(lista: &str) -> String {
        posiciones_fijas(&marcadores(lista)).into_iter().map(|fija| if fija { 'x' } else { '.' }).collect()
    }

    #[test]
    fn cada_marcador_pertenece_a_su_bloque_completo() {
        let lista = marcadores(".I..F..I.F.");
        assert_eq!(bloques_fijos(&lista), vec![(1, 4), (7, 9)]);
        assert_eq!(bloque_de(&lista, 1), Some((1, 4)));
        assert_eq!(bloque_de(&lista, 4), Some((1, 4)));
        assert_eq!(bloque_de(&lista, 9), Some((7, 9)));
        assert_eq!(bloque_de(&lista, 5), None);
        assert_eq!(bloques_fijos(&marcadores("..I..")), vec![(2, 4)]);
        assert_eq!(bloques_fijos(&marcadores(".F.F")), vec![(0, 1), (2, 3)]);
        assert_eq!(bloques_fijos(&marcadores(".I.I.F")), vec![(1, 5)]);
    }

    #[test]
    fn lo_que_esta_entre_inicio_y_fin_queda_fijo() {
        assert_eq!(fijas("..I..F.."), "..xxxx..");
        assert_eq!(fijas(".I.F.I..F."), ".xxx.xxxx.");
        assert_eq!(fijas("IF"), "xx");
        assert_eq!(fijas("...."), "....");
    }

    #[test]
    fn un_inicio_sin_fin_fija_hasta_el_final() {
        assert_eq!(fijas("..I..."), "..xxxx");
    }

    #[test]
    fn un_fin_sin_inicio_fija_desde_el_fin_anterior() {
        assert_eq!(fijas("...F.."), "xxxx..");
        assert_eq!(fijas(".I.F..F."), ".xxxxxx.");
    }

    #[test]
    fn un_inicio_repetido_no_corta_el_bloque() {
        assert_eq!(fijas(".I.I.F."), ".xxxxx.");
    }
}
