# Bundled airport metadata

`airports.tsv` is a generated subset of the
[OurAirports open-data export](https://ourairports.com/data/). It contains
airports with scheduled service and an IATA code, reduced to IATA code, ICAO
code, name, municipality, latitude, and longitude.

OurAirports releases the dataset to the Public Domain. The data comes with no
guarantee of accuracy or fitness for use. Regenerate it with:

```sh
ruby scripts/update_airports.rb
```
