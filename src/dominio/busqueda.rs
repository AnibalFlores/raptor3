//! Busqueda de temas en la lista por titulo, artista, genero o archivo.

use super::duplicados::sin_acento;

/// Texto buscado, separado en palabras. Un tema coincide si contiene todas
/// las palabras en alguno de sus datos, sin importar mayusculas ni acentos.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Busqueda {
    palabras: Vec<String>,
}

impl Busqueda {
    /// None si el texto esta vacio: sin busqueda, la lista funciona como siempre.
    pub fn nueva(texto: &str) -> Option<Self> {
        let palabras: Vec<String> = simplificar(texto).split_whitespace().map(str::to_string).collect();
        (!palabras.is_empty()).then_some(Self { palabras })
    }

    pub fn coincide(&self, datos: &[&str]) -> bool {
        let texto = simplificar(&datos.join(" "));
        self.palabras.iter().all(|palabra| texto.contains(palabra.as_str()))
    }
}

fn simplificar(texto: &str) -> String {
    texto.chars().flat_map(char::to_lowercase).map(sin_acento).collect()
}

/// Proxima coincidencia (avanzando) o anterior (retrocediendo) desde la
/// posicion `desde`, dando la vuelta al llegar a un extremo. Sin posicion de
/// partida, la primera o la ultima. `coincidencias` va ordenada.
pub fn saltar_a_coincidencia(coincidencias: &[usize], desde: Option<usize>, avanzar: bool) -> Option<usize> {
    let (primera, ultima) = (*coincidencias.first()?, *coincidencias.last()?);
    let Some(desde) = desde else {
        return Some(if avanzar { primera } else { ultima });
    };
    if avanzar {
        Some(coincidencias.iter().copied().find(|&indice| indice > desde).unwrap_or(primera))
    } else {
        Some(coincidencias.iter().rev().copied().find(|&indice| indice < desde).unwrap_or(ultima))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn busca_todas_las_palabras_sin_importar_acentos() {
        let busqueda = Busqueda::nueva("  corazon SANZ ").unwrap();
        assert!(busqueda.coincide(&["Corazón partío", "Alejandro Sanz", "Pop"]));
        assert!(!busqueda.coincide(&["Corazón partío", "Otro", "Pop"]));
        assert_eq!(Busqueda::nueva("   "), None);
    }

    #[test]
    fn recorre_las_coincidencias_dando_la_vuelta() {
        let coincidencias = [2, 5, 9];
        assert_eq!(saltar_a_coincidencia(&coincidencias, None, true), Some(2));
        assert_eq!(saltar_a_coincidencia(&coincidencias, None, false), Some(9));
        assert_eq!(saltar_a_coincidencia(&coincidencias, Some(2), true), Some(5));
        assert_eq!(saltar_a_coincidencia(&coincidencias, Some(6), true), Some(9));
        assert_eq!(saltar_a_coincidencia(&coincidencias, Some(9), true), Some(2));
        assert_eq!(saltar_a_coincidencia(&coincidencias, Some(2), false), Some(9));
        assert_eq!(saltar_a_coincidencia(&[], Some(2), false), None);
    }
}
