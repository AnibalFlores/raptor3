//! Temas repetidos en la lista: archivos distintos con el mismo titulo y
//! artista (por ejemplo, el mismo tema en MP3 y en FLAC, o una copia en
//! otra carpeta).

use std::collections::{HashMap, HashSet};

use super::Pista;

/// Un tema repetido: la pista que conviene quitar y la que se conserva.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicado {
    pub conservar: Pista,
    pub quitar: Pista,
}

/// Clave para comparar temas: titulo y artista en minusculas, sin acentos,
/// espacios ni signos. None si no hay titulo.
pub fn clave_de_tema(titulo: &str, artista: &str) -> Option<String> {
    let simplificar = |texto: &str| -> String {
        texto
            .chars()
            .flat_map(char::to_lowercase)
            .map(sin_acento)
            .filter(|letra| letra.is_alphanumeric())
            .collect()
    };
    let titulo = simplificar(titulo);
    (!titulo.is_empty()).then(|| format!("{titulo}|{}", simplificar(artista)))
}

pub(super) fn sin_acento(letra: char) -> char {
    match letra {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        otra => otra,
    }
}

/// Busca los temas repetidos entre `temas` (pista y clave, en el orden de la
/// lista). En cada grupo se conserva, en este orden de preferencia, la pista
/// `protegida` (la que suena), una que no este entre `nuevas` y la primera; las
/// demas se proponen para quitar. Si `solo_nuevas` es true, solo se proponen
/// las que estan entre `nuevas` (al agregar temas no se toca lo que ya habia).
pub fn buscar_duplicados(
    temas: &[(Pista, String)],
    nuevas: &HashSet<Pista>,
    protegida: Option<&Pista>,
    solo_nuevas: bool,
) -> Vec<Duplicado> {
    let mut grupos: HashMap<&str, Vec<&Pista>> = HashMap::new();
    let mut orden: Vec<&str> = Vec::new();
    for (pista, clave) in temas {
        let grupo = grupos.entry(clave.as_str()).or_insert_with(|| {
            orden.push(clave.as_str());
            Vec::new()
        });
        if !grupo.contains(&pista) {
            grupo.push(pista);
        }
    }
    let mut duplicados = Vec::new();
    for clave in orden {
        let grupo = &grupos[clave];
        if grupo.len() < 2 {
            continue;
        }
        let conservar = grupo
            .iter()
            .find(|pista| Some(**pista) == protegida)
            .or_else(|| grupo.iter().find(|pista| !nuevas.contains(**pista)))
            .unwrap_or(&grupo[0]);
        for pista in grupo {
            if pista == conservar || Some(*pista) == protegida || (solo_nuevas && !nuevas.contains(*pista)) {
                continue;
            }
            duplicados.push(Duplicado { conservar: (*conservar).clone(), quitar: (*pista).clone() });
        }
    }
    duplicados
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn tema(archivo: &str, titulo: &str, artista: &str) -> (Pista, String) {
        (Pista::new(archivo), clave_de_tema(titulo, artista).expect("hay titulo"))
    }

    #[test]
    fn la_clave_ignora_mayusculas_acentos_y_signos() {
        assert_eq!(clave_de_tema("Corazón  Partío", "Alejandro Sanz"), clave_de_tema("corazon partio!", "ALEJANDRO SANZ"));
        assert_ne!(clave_de_tema("Fragile", "Sting"), clave_de_tema("Fragile", "Kerli"));
        assert_eq!(clave_de_tema("  ", "Sting"), None);
    }

    #[test]
    fn al_validar_se_conserva_la_primera_o_la_que_suena() {
        let temas = vec![tema("a.mp3", "Tema", "X"), tema("b.mp3", "Otro", "X"), tema("a.flac", "tema", "x"), tema("c.mp3", "TEMA", "X")];
        let ninguna = HashSet::new();
        let duplicados = buscar_duplicados(&temas, &ninguna, None, false);
        assert_eq!(duplicados.len(), 2);
        assert!(duplicados.iter().all(|duplicado| duplicado.conservar == Pista::new("a.mp3")));
        // Si suena la copia, se propone quitar las otras.
        let suena = Pista::new("a.flac");
        let duplicados = buscar_duplicados(&temas, &ninguna, Some(&suena), false);
        assert!(duplicados.iter().all(|duplicado| duplicado.conservar == suena && duplicado.quitar != suena));
        assert_eq!(duplicados.len(), 2);
    }

    #[test]
    fn al_agregar_solo_se_proponen_las_nuevas() {
        // La nueva quedo antes que la que ya estaba: igual se propone quitar la nueva.
        let temas = vec![tema("nueva.flac", "Tema", "X"), tema("vieja.mp3", "Tema", "X"), tema("otra.mp3", "Otro", "Y")];
        let nuevas: HashSet<Pista> = [Pista::new("nueva.flac")].into();
        let duplicados = buscar_duplicados(&temas, &nuevas, None, true);
        assert_eq!(duplicados, vec![Duplicado { conservar: Pista::new("vieja.mp3"), quitar: Pista::new("nueva.flac") }]);
    }
}
