//! Anunciantes: la ficha de quien contrata la publicidad (datos fiscales y
//! de contacto). Las ordenes lo nombran por su nombre comercial, que es la
//! clave de la ficha.

use super::ordenes::simplificar;

/// Pesos del digito verificador del CUIT/CUIL (modulo 11).
const PESOS_DEL_CUIT: [u32; 10] = [5, 4, 3, 2, 7, 6, 5, 4, 3, 2];

/// Ficha de un anunciante.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Anunciante {
    /// Nombre comercial ("Supermercado Sol"): el que usan las ordenes.
    pub nombre: String,
    pub razon_social: String,
    /// CUIT con guiones ("30-71234567-1"); vacio si no se cargo.
    pub cuit: String,
    /// Rubro que toman sus ordenes nuevas si no indican otro.
    pub rubro: String,
    /// Persona de contacto.
    pub contacto: String,
    pub telefono: String,
    pub correo: String,
    pub direccion: String,
    /// Agencia de publicidad que lo representa, si la hay.
    pub agencia: String,
    pub notas: String,
    /// Inactivo: se conserva la ficha pero no admite ordenes nuevas.
    pub activo: bool,
}

impl Anunciante {
    pub fn se_llama(&self, nombre: &str) -> bool {
        simplificar(&self.nombre) == simplificar(nombre)
    }

    /// Valida la ficha y la devuelve normalizada: textos sin espacios de
    /// sobra y el CUIT con guiones.
    pub fn validado(mut self) -> Result<Self, String> {
        for campo in [
            &mut self.nombre,
            &mut self.razon_social,
            &mut self.cuit,
            &mut self.rubro,
            &mut self.contacto,
            &mut self.telefono,
            &mut self.correo,
            &mut self.direccion,
            &mut self.agencia,
            &mut self.notas,
        ] {
            *campo = campo.trim().to_string();
        }
        if self.nombre.is_empty() {
            return Err(tr!("Indique el nombre comercial del anunciante.").into());
        }
        if !self.cuit.is_empty() {
            self.cuit = normalizar_cuit(&self.cuit)?;
        }
        if !self.correo.is_empty() && !correo_valido(&self.correo) {
            return Err(tr!("El correo «{}» no es valido (ejemplo: ventas@empresa.com).", self.correo));
        }
        if !self.telefono.is_empty() && !telefono_valido(&self.telefono) {
            return Err(tr!("El telefono «{}» solo puede tener numeros, espacios, guiones, parentesis y +.", self.telefono));
        }
        Ok(self)
    }

    /// Lo que falta cargar para facturarle (para advertirlo, no impide guardar).
    pub fn faltantes(&self) -> Vec<String> {
        let mut faltan = Vec::new();
        if self.razon_social.is_empty() {
            faltan.push(tr!("razon social"));
        }
        if self.cuit.is_empty() {
            faltan.push("CUIT".into());
        }
        if self.telefono.is_empty() && self.correo.is_empty() {
            faltan.push(tr!("telefono o correo"));
        }
        faltan
    }
}

/// Valida un CUIT/CUIL (11 digitos con digito verificador, con o sin
/// guiones ni espacios) y lo devuelve como "30-71234567-1".
pub fn normalizar_cuit(texto: &str) -> Result<String, String> {
    let digitos: Vec<u32> = texto.chars().filter(|letra| !matches!(letra, '-' | ' ' | '.')).map(|letra| letra.to_digit(10).ok_or(())).collect::<Result<_, _>>().map_err(|()| tr!("El CUIT «{texto}» solo puede tener numeros y guiones.", texto = texto))?;
    if digitos.len() != 11 {
        return Err(tr!("El CUIT «{texto}» debe tener 11 digitos (tiene {}).", digitos.len(), texto = texto));
    }
    let suma: u32 = digitos.iter().zip(PESOS_DEL_CUIT).map(|(digito, peso)| digito * peso).sum();
    let esperado = match 11 - suma % 11 {
        11 => 0,
        10 => return Err(tr!("El CUIT «{texto}» no es valido: revise los numeros.", texto = texto)),
        resto => resto,
    };
    if digitos[10] != esperado {
        return Err(tr!("El CUIT «{texto}» no es valido: el digito verificador deberia ser {esperado}.", texto = texto, esperado = esperado));
    }
    let texto: String = digitos.iter().map(|digito| char::from_digit(*digito, 10).unwrap_or('0')).collect();
    Ok(format!("{}-{}-{}", &texto[..2], &texto[2..10], &texto[10..]))
}

/// Un correo con usuario, @ y dominio con punto, sin espacios.
fn correo_valido(correo: &str) -> bool {
    let Some((usuario, dominio)) = correo.split_once('@') else { return false };
    !usuario.is_empty()
        && !correo.contains(char::is_whitespace)
        && !dominio.contains('@')
        && dominio.split('.').count() >= 2
        && dominio.split('.').all(|parte| !parte.is_empty())
}

fn telefono_valido(telefono: &str) -> bool {
    telefono.chars().any(|letra| letra.is_ascii_digit())
        && telefono.chars().all(|letra| letra.is_ascii_digit() || matches!(letra, ' ' | '-' | '(' | ')' | '+' | '/'))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn valida_el_cuit_con_su_digito_verificador() {
        assert_eq!(normalizar_cuit("20-12345678-6"), Ok("20-12345678-6".into()));
        assert_eq!(normalizar_cuit("20123456786"), Ok("20-12345678-6".into()));
        assert_eq!(normalizar_cuit(" 30 71234567 1 "), Ok("30-71234567-1".into()));
        assert!(normalizar_cuit("20-12345678-5").unwrap_err().contains("deberia ser 6"));
        assert!(normalizar_cuit("20-1234567-6").unwrap_err().contains("11 digitos"));
        assert!(normalizar_cuit("20-ABCDEFGH-6").unwrap_err().contains("solo puede tener numeros"));
    }

    #[test]
    fn normaliza_la_ficha_y_advierte_lo_que_falta() {
        let ficha = Anunciante {
            nombre: "  Supermercado Sol ".into(),
            cuit: "30712345671".into(),
            correo: "ventas@sol.com.ar".into(),
            telefono: "(011) 4555-1234".into(),
            activo: true,
            ..Anunciante::default()
        }
        .validado()
        .unwrap();
        assert_eq!((ficha.nombre.as_str(), ficha.cuit.as_str()), ("Supermercado Sol", "30-71234567-1"));
        assert_eq!(ficha.faltantes(), ["razon social"]);
        assert!(ficha.se_llama("supermercado SOL"));

        let sin_nombre = Anunciante { nombre: " ".into(), ..Anunciante::default() };
        assert!(sin_nombre.validado().is_err());
        for correo in ["ventas", "ventas@sol", "ven tas@sol.com", "@sol.com", "a@b@c.com", "a@sol..com"] {
            let ficha = Anunciante { nombre: "X".into(), correo: correo.into(), ..Anunciante::default() };
            assert!(ficha.validado().is_err(), "{correo}");
        }
        let telefono = Anunciante { nombre: "X".into(), telefono: "llamar".into(), ..Anunciante::default() };
        assert!(telefono.validado().is_err());
        assert_eq!(Anunciante { nombre: "X".into(), ..Anunciante::default() }.faltantes(), ["razon social", "CUIT", "telefono o correo"]);
    }
}
