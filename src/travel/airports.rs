use super::{AirportMetadata, TravelAirport};
use std::collections::HashMap;
use std::sync::OnceLock;

const AIRPORTS_TSV: &str = include_str!("../../assets/airports.tsv");

pub(super) fn airport_for_code(code: &str) -> TravelAirport {
    TravelAirport {
        code: code.to_string(),
        metadata: airport_database().get(code).cloned(),
    }
}

fn airport_database() -> &'static HashMap<String, AirportMetadata> {
    static AIRPORTS: OnceLock<HashMap<String, AirportMetadata>> = OnceLock::new();
    AIRPORTS.get_or_init(|| {
        let mut airports = HashMap::new();
        for (line_index, line) in AIRPORTS_TSV.lines().enumerate() {
            let columns = line.split('\t').collect::<Vec<_>>();
            assert_eq!(
                columns.len(),
                6,
                "invalid bundled airport row {}",
                line_index + 1
            );
            let metadata = AirportMetadata {
                iata_code: columns[0].to_string(),
                icao_code: nonempty(columns[1]),
                name: columns[2].to_string(),
                municipality: nonempty(columns[3]),
                latitude: columns[4]
                    .parse()
                    .expect("bundled airport latitude must be numeric"),
                longitude: columns[5]
                    .parse()
                    .expect("bundled airport longitude must be numeric"),
            };
            airports
                .entry(metadata.iata_code.clone())
                .or_insert_with(|| metadata.clone());
            if let Some(icao_code) = &metadata.icao_code {
                airports
                    .entry(icao_code.clone())
                    .or_insert_with(|| metadata.clone());
            }
        }
        airports
    })
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}
