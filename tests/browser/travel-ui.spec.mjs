import {expect, test} from "@playwright/test";
import {spawn, spawnSync} from "node:child_process";
import {mkdtempSync, rmSync} from "node:fs";
import {tmpdir} from "node:os";
import {join} from "node:path";
import {createInterface} from "node:readline";

const repository = process.cwd();
const binary = join(repository, "target", "debug", "icalctl");
let temporaryHome;
let serverProcess;
let serverInfo;
let serverStderr = "";

test.describe.configure({mode: "serial"});

test.beforeAll(async () => {
  const build = spawnSync("cargo", ["build"], {
    cwd: repository,
    encoding: "utf8",
    timeout: 120_000,
  });
  expect(build.status, build.stderr).toBe(0);

  temporaryHome = mkdtempSync(join(tmpdir(), "icalctl-browser-test-"));
  const environment = {...process.env, HOME: temporaryHome};
  for (const arguments_ of [
    ["config", "init"],
    ["config", "set", "travel.server.open_browser", "false"],
    ["config", "set", "flightaware.enabled", "false"],
  ]) {
    const command = spawnSync(binary, arguments_, {
      cwd: repository,
      env: environment,
      encoding: "utf8",
      timeout: 15_000,
    });
    expect(command.status, command.stderr).toBe(0);
  }

  serverProcess = spawn(binary, ["--json", "travel", "serve"], {
    cwd: repository,
    env: environment,
    stdio: ["ignore", "pipe", "pipe"],
  });
  serverProcess.stderr.on("data", (chunk) => {
    serverStderr += chunk.toString();
  });
  serverInfo = JSON.parse(await firstLine(serverProcess.stdout));
  expect(serverInfo.read_only).toBe(true);
  expect(serverInfo.bind).toBe("127.0.0.1");
});

test.afterAll(() => {
  if (serverProcess && serverProcess.exitCode === null) {
    serverProcess.kill("SIGTERM");
  }
  if (temporaryHome) {
    rmSync(temporaryHome, {recursive: true, force: true});
  }
});

test("renders, filters, selects, and switches projection without refetching", async ({page}) => {
  let apiCalls = 0;
  const consoleErrors = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });
  page.on("pageerror", (error) => consoleErrors.push(error.message));

  await page.route("**/assets/test-style.json", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        version: 8,
        sources: {},
        layers: [
          {
            id: "background",
            type: "background",
            paint: {"background-color": "#0b1317"},
          },
        ],
      }),
    }),
  );
  await page.route("**/api/travel**", (route) => {
    apiCalls += 1;
    const requestUrl = new URL(route.request().url());
    const payload = requestUrl.searchParams.has("start")
      ? emptyPayload(requestUrl.searchParams.get("start"), requestUrl.searchParams.get("end"))
      : travelPayload();
    return route.fulfill({contentType: "application/json", body: JSON.stringify(payload)});
  });

  await page.goto(serverInfo.url, {waitUntil: "domcontentloaded"});
  await expect(page).toHaveURL(new RegExp(`^http://127\\.0\\.0\\.1:${serverInfo.port}/$`));
  await expect(page.locator(".trip-card")).toHaveCount(4);
  await expect(page.locator("#trip-count")).toHaveText("4");
  await expect(page.locator(".trip-card").nth(0)).toContainText("HO1607");
  await expect(page.locator(".trip-card").nth(1)).toContainText("AY1415");
  await expect(page.locator(".trip-card").nth(2)).toContainText("XX9");
  await expect(page.locator(".trip-card").nth(3)).toContainText("JL2");
  await expect(page.locator(".trip-card").nth(0)).toContainText("09:25 (+08:00)");
  await expect(page.locator(".trip-card").nth(0)).toContainText("14:00 (+03:00)");
  await expect(page.locator("body")).toHaveAttribute("data-map-ready", "true", {timeout: 20_000});
  await expect(page.locator("body")).toHaveAttribute("data-map-projection", "globe");
  await expect(page.locator("body")).toHaveAttribute("data-route-count", "3");
  await expect(page.locator(".maplibregl-canvas")).toBeVisible();
  await expect(page.locator(".travel-marker")).toHaveCount(7);
  const routeGeometry = JSON.parse(await page.locator("body").getAttribute("data-route-geometry"));
  expect(routeGeometry.map((route) => route.legIndex)).toEqual([0, 1, 3]);
  expect(routeGeometry[0].start[0]).toBeCloseTo(121.8083, 4);
  expect(routeGeometry[0].start[1]).toBeCloseTo(31.1443, 4);
  expect(routeGeometry[0].end[0]).toBeCloseTo(24.9633, 4);
  expect(routeGeometry[0].end[1]).toBeCloseTo(60.3172, 4);
  expect(routeGeometry[2].start[0]).toBeCloseTo(140.3929, 4);
  expect(routeGeometry[2].start[1]).toBeCloseTo(35.7767, 4);
  expect(routeGeometry[2].end[0]).toBeGreaterThan(180);
  expect(routeGeometry[2].end[0] - 360).toBeCloseTo(-122.375, 4);
  expect(routeGeometry[2].end[1]).toBeCloseTo(37.6213, 4);
  expect(apiCalls).toBe(1);

  await expect(page.locator("#warning-list")).toContainText("<img id=warning-xss>");
  await expect(page.locator("#warning-xss")).toHaveCount(0);
  await expect(page.locator("#calendar-xss")).toHaveCount(0);

  await page.locator(".trip-card").nth(1).click();
  await expect(page.locator("body")).toHaveAttribute("data-selected-leg", "1");
  await expect(page.locator("#detail-heading")).toContainText("AY1415");
  await expect(page.locator("#detail-content")).toContainText("<img id=calendar-xss>");
  await expect(page.locator("#detail-content")).toContainText("Estimated arrival updated safely");

  await page.locator('[data-projection="mercator"]').click();
  await expect(page.locator("body")).toHaveAttribute("data-projection", "mercator");
  await expect(page.locator("body")).toHaveAttribute("data-map-projection", "mercator");
  expect(apiCalls).toBe(1);
  await page.locator('[data-projection="globe"]').click();
  await expect(page.locator("body")).toHaveAttribute("data-projection", "globe");
  await expect(page.locator("body")).toHaveAttribute("data-map-projection", "globe");
  expect(apiCalls).toBe(1);

  await page.locator("#start-date").fill("2026-09-10");
  await page.locator("#end-date").fill("2026-09-01");
  await page.locator("#refresh-button").click();
  await expect(page.locator("#form-error")).toContainText("Start date must be on or before end date");
  expect(apiCalls).toBe(1);

  await page.locator("#start-date").fill("2026-09-01");
  await page.locator("#end-date").fill("2026-09-30");
  await page.locator("#refresh-button").click();
  await expect(page.locator("#empty-state")).toBeVisible();
  await expect(page.locator(".trip-card")).toHaveCount(0);
  expect(apiCalls).toBe(2);

  await page.setViewportSize({width: 500, height: 820});
  await expect(page.locator(".topbar")).toBeVisible();
  await expect(page.locator(".map-panel")).toBeVisible();
  await expect(page.locator(".itinerary-panel")).toBeVisible();
  expect(consoleErrors, `${consoleErrors.join("\n")}\n${serverStderr}`).toEqual([]);
});

function firstLine(stream) {
  return new Promise((resolve, reject) => {
    const lines = createInterface({input: stream});
    const timeout = setTimeout(() => {
      lines.close();
      reject(new Error(`travel server did not start: ${serverStderr}`));
    }, 15_000);
    lines.once("line", (line) => {
      clearTimeout(timeout);
      lines.close();
      resolve(line);
    });
    stream.once("error", reject);
  });
}

function emptyPayload(start, end) {
  return {
    schema_version: 1,
    generated_at: "2026-07-12T12:00:00Z",
    range: {start, end, end_inclusive: true},
    calendar_ids: [],
    map: {projection: "globe", style_url: "/assets/test-style.json"},
    legs: [],
    warnings: [],
  };
}

function travelPayload() {
  return {
    schema_version: 1,
    generated_at: "2026-07-12T12:00:00Z",
    range: {start: "2026-07-12", end: "2026-10-09", end_inclusive: true},
    calendar_ids: ["CAL-TRAVEL"],
    map: {projection: "globe", style_url: "/assets/test-style.json"},
    warnings: [
      {
        kind: "fixture_warning",
        event_id: "EVENT-2",
        message: "<img id=warning-xss> remains plain text",
      },
      {
        kind: "unknown_airport",
        event_id: "EVENT-3",
        message: "ZZZ has no bundled coordinates",
      },
    ],
    legs: [
      leg({
        eventId: "EVENT-1",
        flight: "HO1607",
        from: airport("PVG", 31.1443, 121.8083, "Shanghai Pudong International Airport"),
        to: airport("HEL", 60.3172, 24.9633, "Helsinki Vantaa Airport"),
        departure: "2026-07-13T09:25:00+08:00",
        arrival: "2026-07-13T14:00:00+03:00",
        calendar: "Travel",
        live: liveStatus("en_route", "En route", "fresh"),
      }),
      leg({
        eventId: "EVENT-2",
        flight: "AY1415",
        from: airport("HEL", 60.3172, 24.9633, "Helsinki Vantaa Airport"),
        to: airport("FRA", 50.0379, 8.5622, "Frankfurt Airport"),
        departure: "2026-07-14T07:40:00+03:00",
        arrival: "2026-07-14T09:20:00+02:00",
        calendar: "<img id=calendar-xss>",
        live: liveStatus("scheduled", "Estimated arrival updated safely", "stale"),
      }),
      leg({
        eventId: "EVENT-3",
        flight: "XX9",
        from: {code: "ZZZ", metadata: null},
        to: airport("JFK", 40.6413, -73.7781, "John F. Kennedy International Airport"),
        departure: "2026-07-16T10:00:00+01:00",
        arrival: "2026-07-16T13:30:00-04:00",
        calendar: "Travel",
        live: null,
      }),
      leg({
        eventId: "EVENT-4",
        flight: "JL2",
        from: airport("NRT", 35.7767, 140.3929, "Narita International Airport"),
        to: airport("SFO", 37.6213, -122.375, "San Francisco International Airport"),
        departure: "2026-07-18T16:30:00+09:00",
        arrival: "2026-07-18T09:20:00-07:00",
        calendar: "Travel",
        live: null,
      }),
    ],
  };
}

function leg({eventId, flight, from, to, departure, arrival, calendar, live}) {
  return {
    flight_number: flight,
    route: `${from.code} to ${to.code}`,
    departure_airport: from,
    arrival_airport: to,
    departure: {scheduled: departure, utc: new Date(departure).toISOString()},
    arrival: {scheduled: arrival, utc: new Date(arrival).toISOString()},
    live_status: live,
    source: {
      event_id: eventId,
      occurrence_date: null,
      title: `Flight ${flight}: ${from.code} to ${to.code}`,
      calendar,
      calendar_id: "CAL-TRAVEL",
    },
  };
}

function airport(code, latitude, longitude, name) {
  return {
    code,
    metadata: {
      iata_code: code,
      icao_code: null,
      name,
      municipality: null,
      latitude,
      longitude,
    },
  };
}

function liveStatus(status, description, freshnessState) {
  return {
    provider: "flightaware",
    provider_flight_id: `FA-${status}`,
    status,
    description,
    scheduled_departure: null,
    estimated_departure: "2026-07-13T01:30:00Z",
    actual_departure: null,
    scheduled_arrival: null,
    estimated_arrival: "2026-07-13T11:10:00Z",
    actual_arrival: null,
    departure_delay_seconds: 300,
    arrival_delay_seconds: 600,
    departure_terminal: "2",
    departure_gate: "D71",
    arrival_terminal: "2",
    arrival_gate: "32",
    tracking_ended: false,
    diverted: false,
    current_position: null,
    freshness: {
      state: freshnessState,
      fetched_at: "2026-07-12T11:58:00Z",
      expires_at: "2026-07-12T12:03:00Z",
    },
  };
}
