/// The 8 categories the unit converter supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Length,
    Mass,
    Temperature,
    Area,
    Volume,
    Speed,
    Time,
    Data,
}

pub const CATEGORIES: [Category; 8] = [
    Category::Length,
    Category::Mass,
    Category::Temperature,
    Category::Area,
    Category::Volume,
    Category::Speed,
    Category::Time,
    Category::Data,
];

pub struct Unit {
    pub symbol: &'static str,
    /// How many base units one of this unit equals (temperature isn't a linear
    /// conversion, so it's special-cased in `Category::units` instead).
    to_base: f64,
}

impl Category {
    pub fn units(self) -> &'static [Unit] {
        match self {
            Category::Length => &[
                Unit {
                    symbol: "m",
                    to_base: 1.0,
                },
                Unit {
                    symbol: "km",
                    to_base: 1000.0,
                },
                Unit {
                    symbol: "cm",
                    to_base: 0.01,
                },
                Unit {
                    symbol: "mm",
                    to_base: 0.001,
                },
                Unit {
                    symbol: "mile",
                    to_base: 1609.344,
                },
                Unit {
                    symbol: "yard",
                    to_base: 0.9144,
                },
                Unit {
                    symbol: "foot",
                    to_base: 0.3048,
                },
                Unit {
                    symbol: "inch",
                    to_base: 0.0254,
                },
            ],
            Category::Mass => &[
                Unit {
                    symbol: "kg",
                    to_base: 1.0,
                },
                Unit {
                    symbol: "g",
                    to_base: 0.001,
                },
                Unit {
                    symbol: "mg",
                    to_base: 0.000_001,
                },
                Unit {
                    symbol: "lb",
                    to_base: 0.453_592_37,
                },
                Unit {
                    symbol: "oz",
                    to_base: 0.028_349_523_125,
                },
                Unit {
                    symbol: "t",
                    to_base: 1000.0,
                },
            ],
            Category::Temperature => &[
                Unit {
                    symbol: "°C",
                    to_base: 0.0,
                },
                Unit {
                    symbol: "°F",
                    to_base: 0.0,
                },
                Unit {
                    symbol: "K",
                    to_base: 0.0,
                },
            ],
            Category::Area => &[
                Unit {
                    symbol: "m²",
                    to_base: 1.0,
                },
                Unit {
                    symbol: "km²",
                    to_base: 1_000_000.0,
                },
                Unit {
                    symbol: "cm²",
                    to_base: 0.000_1,
                },
                Unit {
                    symbol: "ha",
                    to_base: 10_000.0,
                },
                Unit {
                    symbol: "acre",
                    to_base: 4046.8564224,
                },
                Unit {
                    symbol: "ft²",
                    to_base: 0.092_903_04,
                },
            ],
            Category::Volume => &[
                Unit {
                    symbol: "L",
                    to_base: 1.0,
                },
                Unit {
                    symbol: "mL",
                    to_base: 0.001,
                },
                Unit {
                    symbol: "m³",
                    to_base: 1000.0,
                },
                Unit {
                    symbol: "gal (US)",
                    to_base: 3.785_411_784,
                },
                Unit {
                    symbol: "qt (US)",
                    to_base: 0.946_352_946,
                },
                Unit {
                    symbol: "cup (US)",
                    to_base: 0.236_588_236_5,
                },
            ],
            Category::Speed => &[
                Unit {
                    symbol: "m/s",
                    to_base: 1.0,
                },
                Unit {
                    symbol: "km/h",
                    to_base: 1.0 / 3.6,
                },
                Unit {
                    symbol: "mph",
                    to_base: 0.447_04,
                },
                Unit {
                    symbol: "knot",
                    to_base: 0.514_444_444,
                },
            ],
            Category::Time => &[
                Unit {
                    symbol: "s",
                    to_base: 1.0,
                },
                Unit {
                    symbol: "min",
                    to_base: 60.0,
                },
                Unit {
                    symbol: "h",
                    to_base: 3600.0,
                },
                Unit {
                    symbol: "day",
                    to_base: 86_400.0,
                },
            ],
            Category::Data => &[
                Unit {
                    symbol: "B",
                    to_base: 1.0,
                },
                Unit {
                    symbol: "KB",
                    to_base: 1024.0,
                },
                Unit {
                    symbol: "MB",
                    to_base: 1024.0 * 1024.0,
                },
                Unit {
                    symbol: "GB",
                    to_base: 1024.0 * 1024.0 * 1024.0,
                },
                Unit {
                    symbol: "TB",
                    to_base: 1024.0 * 1024.0 * 1024.0 * 1024.0,
                },
            ],
        }
    }
}

/// Converts `value` within `category` from the unit at index `from` to the one at
/// index `to`. Returns `None` if either index is out of range.
pub fn convert(category: Category, value: f64, from: usize, to: usize) -> Option<f64> {
    let units = category.units();
    let from_unit = units.get(from)?;
    let to_unit = units.get(to)?;

    if category == Category::Temperature {
        let celsius = match from_unit.symbol {
            "°C" => value,
            "°F" => (value - 32.0) * 5.0 / 9.0,
            "K" => value - 273.15,
            _ => unreachable!(),
        };
        return Some(match to_unit.symbol {
            "°C" => celsius,
            "°F" => celsius * 9.0 / 5.0 + 32.0,
            "K" => celsius + 273.15,
            _ => unreachable!(),
        });
    }

    Some(value * from_unit.to_base / to_unit.to_base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_km_to_miles() {
        let km_idx = 1;
        let mile_idx = 4;
        let result = convert(Category::Length, 1.0, km_idx, mile_idx).unwrap();
        assert!((result - 0.621_371).abs() < 0.001);
    }

    #[test]
    fn converts_celsius_to_fahrenheit() {
        let result = convert(Category::Temperature, 0.0, 0, 1).unwrap();
        assert!((result - 32.0).abs() < 0.0001);
        let result = convert(Category::Temperature, 100.0, 0, 1).unwrap();
        assert!((result - 212.0).abs() < 0.0001);
    }

    #[test]
    fn converts_celsius_to_kelvin() {
        let result = convert(Category::Temperature, 0.0, 0, 2).unwrap();
        assert!((result - 273.15).abs() < 0.0001);
    }

    #[test]
    fn converts_gb_to_mb() {
        let result = convert(Category::Data, 1.0, 3, 2).unwrap();
        assert!((result - 1024.0).abs() < 0.001);
    }

    #[test]
    fn out_of_range_index_is_none() {
        assert!(convert(Category::Length, 1.0, 99, 0).is_none());
    }
}
