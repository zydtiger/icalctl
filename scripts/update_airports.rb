#!/usr/bin/env ruby

require "csv"
require "fileutils"
require "net/http"
require "uri"

SOURCE = URI("https://raw.githubusercontent.com/davidmegginson/ourairports-data/main/airports.csv")
OUTPUT = File.expand_path("../assets/airports.tsv", __dir__)

response = Net::HTTP.get_response(SOURCE)
abort("failed to download #{SOURCE}: HTTP #{response.code}") unless response.is_a?(Net::HTTPSuccess)

rows = CSV.parse(response.body, headers: true).each_with_object([]) do |row, selected|
  iata = row["iata_code"].to_s.strip.upcase
  icao = row["icao_code"].to_s.strip.upcase
  next if row["scheduled_service"] != "yes" || iata.empty?

  selected << [
    iata,
    icao,
    row["name"],
    row["municipality"],
    row["latitude_deg"],
    row["longitude_deg"],
  ].map { |value| value.to_s.gsub(/[\t\r\n]/, " ").strip }
end

rows.sort_by! { |row| [row[0], row[1]] }
FileUtils.mkdir_p(File.dirname(OUTPUT)) unless Dir.exist?(File.dirname(OUTPUT))
File.write(OUTPUT, rows.map { |row| row.join("\t") }.join("\n") << "\n")
warn("wrote #{rows.length} airports to #{OUTPUT}")
