//! Formato en que la interfaz muestra (y lee) las fechas: dia/mes/año,
//! mes/dia/año o año-mes-dia. Se elige en el menu Ventana, como el idioma,
//! y rige para todo el programa: el reloj, la proxima tanda, la columna Sono
//! de la lista y del catalogo, el panel de detalle y el editor. Los registros
//! y los archivos guardados siguen en ISO 8601, sin importar esta eleccion.

use std::sync::atomic::{AtomicU8, Ordering};

/// Orden de dia, mes y año.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormatoDeFecha {
    /// 23/11/2026
    #[default]
    DiaMesAnio,
    /// 11/23/2026
    MesDiaAnio,
    /// 2026-11-23
    AnioMesDia,
}

impl FormatoDeFecha {
    /// Todos, en el orden en que se ofrecen.
    pub const TODOS: [Self; 3] = [Self::DiaMesAnio, Self::MesDiaAnio, Self::AnioMesDia];

    /// Clave que se guarda en las preferencias.
    pub fn codigo(self) -> &'static str {
        match self {
            Self::DiaMesAnio => "dma",
            Self::MesDiaAnio => "mda",
            Self::AnioMesDia => "amd",
        }
    }

    pub fn desde_codigo(codigo: &str) -> Option<Self> {
        Self::TODOS.into_iter().find(|formato| formato.codigo() == codigo.trim())
    }

    /// Fecha completa para `strftime`/`strptime`: "%d/%m/%Y".
    pub fn completa(self) -> &'static str {
        match self {
            Self::DiaMesAnio => "%d/%m/%Y",
            Self::MesDiaAnio => "%m/%d/%Y",
            Self::AnioMesDia => "%Y-%m-%d",
        }
    }

    /// Fecha con el año corto, para las columnas angostas: "%d/%m/%y". En
    /// año-mes-dia el año va completo (asi no se confunde con el dia).
    pub fn corta(self) -> &'static str {
        match self {
            Self::DiaMesAnio => "%d/%m/%y",
            Self::MesDiaAnio => "%m/%d/%y",
            Self::AnioMesDia => "%Y-%m-%d",
        }
    }

    /// Dia y mes, sin el año: "%d/%m".
    pub fn dia_y_mes(self) -> &'static str {
        match self {
            Self::DiaMesAnio => "%d/%m",
            Self::MesDiaAnio => "%m/%d",
            Self::AnioMesDia => "%m-%d",
        }
    }

    /// Como se escribe una fecha completa, en el idioma de la interfaz:
    /// "dd/mm/aaaa" o "dd/mm/yyyy".
    pub fn para_escribir(self) -> String {
        let anio = if crate::idioma::en_ingles() { "yyyy" } else { "aaaa" };
        match self {
            Self::DiaMesAnio => format!("dd/mm/{anio}"),
            Self::MesDiaAnio => format!("mm/dd/{anio}"),
            Self::AnioMesDia => format!("{anio}-mm-dd"),
        }
    }

    /// Una fecha de ejemplo en este formato ("01/10/2026"), para las ayudas.
    pub fn ejemplo(self) -> String {
        jiff::civil::date(2026, 10, 1).strftime(self.completa()).to_string()
    }

    /// Para el menu: el patron y un ejemplo ("dd/mm/aaaa (23/11/2026)").
    pub fn nombre(self) -> String {
        let ejemplo = jiff::civil::date(2026, 11, 23).strftime(self.completa()).to_string();
        format!("{} ({ejemplo})", self.para_escribir())
    }
}

static ACTUAL: AtomicU8 = AtomicU8::new(0);

/// Formato en que se muestran las fechas.
pub fn actual() -> FormatoDeFecha {
    FormatoDeFecha::TODOS.get(usize::from(ACTUAL.load(Ordering::Relaxed))).copied().unwrap_or_default()
}

pub fn establecer(formato: FormatoDeFecha) {
    let indice = FormatoDeFecha::TODOS.iter().position(|otro| *otro == formato).unwrap_or(0);
    ACTUAL.store(indice as u8, Ordering::Relaxed);
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn cada_formato_ordena_dia_mes_y_anio() {
        let dia = jiff::civil::date(2026, 11, 3);
        let ver = |patron: &str| dia.strftime(patron).to_string();
        assert_eq!(ver(FormatoDeFecha::DiaMesAnio.completa()), "03/11/2026");
        assert_eq!(ver(FormatoDeFecha::MesDiaAnio.completa()), "11/03/2026");
        assert_eq!(ver(FormatoDeFecha::AnioMesDia.completa()), "2026-11-03");
        assert_eq!(ver(FormatoDeFecha::DiaMesAnio.corta()), "03/11/26");
        assert_eq!(ver(FormatoDeFecha::MesDiaAnio.dia_y_mes()), "11/03");
        for formato in FormatoDeFecha::TODOS {
            assert_eq!(FormatoDeFecha::desde_codigo(formato.codigo()), Some(formato));
        }
        assert_eq!(FormatoDeFecha::desde_codigo("otro"), None);
    }
}
