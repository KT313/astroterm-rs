//! The city table (`cities.csv`, from GeoNames): name, population, country, timezone, latitude, longitude.

use super::CatalogError;

/// A city with its location in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct City {
    pub name: &'static str,
    pub population: u64,
    pub latitude: f64,
    pub longitude: f64,
}

/// Parse the CSV rows after the header line.
pub fn parse_cities(text: &'static str) -> Result<Vec<City>, CatalogError> {
    text.lines()
        .enumerate()
        .skip(1)
        .map(|(index, line)| (index + 1, line.trim_end()))
        .filter(|(_, line)| !line.is_empty())
        .map(|(line_number, line)| parse_city_line(line).ok_or(CatalogError::MalformedCity { line: line_number }))
        .collect()
}

/// The city called `name` (ignoring case and surrounding whitespace). If several cities share the name, the one with
/// the largest population wins, e.g. London, UK over London, Ontario.
pub fn find_city<'a>(cities: &'a [City], name: &str) -> Option<&'a City> {
    let wanted = normalize_city_name(name);
    cities
        .iter()
        .filter(|city| normalize_city_name(city.name) == wanted)
        .max_by_key(|city| city.population)
}

fn parse_city_line(line: &'static str) -> Option<City> {
    let mut fields = line.split(',');
    let name = fields.next()?;
    let population = fields.next()?.parse().ok()?;
    let _country_code = fields.next()?;
    let _timezone = fields.next()?;
    let latitude = fields.next()?.parse().ok()?;
    let longitude = fields.next()?.parse().ok()?;
    Some(City {
        name,
        population,
        latitude,
        longitude,
    })
}

fn normalize_city_name(name: &str) -> String {
    name.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CITIES_TEXT;

    fn embedded_cities() -> Vec<City> {
        parse_cities(CITIES_TEXT).expect("embedded cities parse")
    }

    fn location_of(cities: &[City], name: &str) -> Option<(&'static str, f64, f64)> {
        find_city(cities, name).map(|city| (city.name, city.latitude, city.longitude))
    }

    #[test]
    fn finds_reference_cities() {
        let cities = embedded_cities();
        assert_eq!(location_of(&cities, "Tunis"), Some(("Tunis", 36.81897, 10.16579)));
        assert_eq!(location_of(&cities, "Boston"), Some(("Boston", 42.35843, -71.05977)));
        assert_eq!(location_of(&cities, "Lisbon"), Some(("Lisbon", 38.72509, -9.1498)));
        assert_eq!(
            location_of(&cities, "Rio de Janeiro"),
            Some(("Rio de Janeiro", -22.90642, -43.18223))
        );
        assert_eq!(
            location_of(&cities, "Thủ Dầu Một"),
            Some(("Thủ Dầu Một", 10.9804, 106.6519))
        );
    }

    #[test]
    fn largest_city_wins_and_lookup_ignores_case_and_spaces() {
        let cities = embedded_cities();
        assert_eq!(location_of(&cities, "London"), Some(("London", 51.50853, -0.12574)));
        assert_eq!(location_of(&cities, "  lONDON "), Some(("London", 51.50853, -0.12574)));
    }

    #[test]
    fn unknown_and_small_places_are_not_found() {
        let cities = embedded_cities();
        assert_eq!(location_of(&cities, "NonexistentCity"), None);
        assert_eq!(location_of(&cities, "Nantucket"), None);
        assert_eq!(location_of(&cities, "city_name"), None); // the header is not a city
    }

    #[test]
    fn rejects_malformed_rows() {
        let result = parse_cities("header\nGood,1,XX,Zone,1.0,2.0\nBad,many,XX,Zone,1.0,2.0");
        assert_eq!(result, Err(CatalogError::MalformedCity { line: 3 }));
    }
}
