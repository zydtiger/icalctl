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
  await expect(page.locator("#form-error")).toBeHidden();
  const workspaceBounds = await page.locator(".workspace").boundingBox();
  const viewportHeight = await page.evaluate(() => window.innerHeight);
  expect(workspaceBounds).not.toBeNull();
  expect(Math.abs(workspaceBounds.y + workspaceBounds.height - viewportHeight)).toBeLessThanOrEqual(1);
  await expect(page.locator(".trip-card")).toHaveCount(4);
  await expect(page.locator("#trip-count")).toHaveText("4");
  const panelLayout = await page.evaluate(() => {
    const bounds = (selector) => {
      const rect = document.querySelector(selector).getBoundingClientRect();
      return {top: rect.top, bottom: rect.bottom, height: rect.height};
    };
    const tripList = document.querySelector("#trip-list");
    const detailContent = document.querySelector("#detail-content");
    return {
      selector: bounds("#trip-selector"),
      warning: bounds("#warning-panel"),
      detail: bounds("#detail-panel"),
      tripList: {clientHeight: tripList.clientHeight, scrollHeight: tripList.scrollHeight},
      detailContent: {
        clientHeight: detailContent.clientHeight,
        scrollHeight: detailContent.scrollHeight,
      },
    };
  });
  expect(panelLayout.selector.bottom).toBeLessThanOrEqual(panelLayout.warning.top);
  expect(panelLayout.warning.bottom).toBeLessThanOrEqual(panelLayout.detail.top);
  expect(panelLayout.tripList.clientHeight).toBeGreaterThanOrEqual(100);
  expect(panelLayout.tripList.scrollHeight).toBeGreaterThan(panelLayout.tripList.clientHeight);
  expect(panelLayout.detailContent.clientHeight).toBeGreaterThanOrEqual(60);
  expect(panelLayout.detailContent.scrollHeight).toBeGreaterThan(panelLayout.detailContent.clientHeight);
  await expect(page.locator(".trip-card").nth(0)).toContainText("D83229");
  await expect(page.locator(".trip-card").nth(1)).toContainText("AY1415");
  await expect(page.locator(".trip-card").nth(2)).toContainText("XX9");
  await expect(page.locator(".trip-card").nth(3)).toContainText("JL2");
  await expect(page.locator(".status-chip").nth(0)).toHaveText("En route");
  await expect(page.locator(".status-chip").nth(1)).toHaveText("Scheduled");
  await expect(page.locator(".status-chip").nth(2)).toHaveText("Calendar");
  await expect(page.locator(".trip-card").nth(0)).toContainText("09:25 (+08:00)");
  await expect(page.locator(".trip-card").nth(0)).toContainText("14:00 (+03:00)");
  const badge = page.locator(".flight-badge").first();
  await expect(badge.locator(".flight-carrier")).toHaveText("D8");
  await expect(badge.locator(".flight-number")).toHaveText("3229");
  const badgeBounds = await badge.boundingBox();
  const carrierBounds = await textBounds(badge.locator(".flight-carrier"));
  const numberBounds = await textBounds(badge.locator(".flight-number"));
  expect(badgeBounds).not.toBeNull();
  expect(carrierBounds).not.toBeNull();
  expect(numberBounds).not.toBeNull();
  const badgeCenter = badgeBounds.x + badgeBounds.width / 2;
  expect(Math.abs(carrierBounds.x + carrierBounds.width / 2 - badgeCenter)).toBeLessThanOrEqual(1);
  expect(Math.abs(numberBounds.x + numberBounds.width / 2 - badgeCenter)).toBeLessThanOrEqual(1);
  await expect(page.locator("body")).toHaveAttribute("data-map-ready", "true", {timeout: 20_000});
  await expect(page.locator("body")).toHaveAttribute("data-map-projection", "globe");
  await expect(page.locator("body")).toHaveAttribute("data-route-count", "3");
  await expect(page.locator(".maplibregl-canvas")).toBeVisible();
  await expect(page.locator(".travel-marker")).toHaveCount(7);
  await page.locator('.travel-marker.arrival[data-leg-index="3"]').click();
  await expect(page.locator("body")).toHaveAttribute("data-selected-leg", "3");
  const revealedCard = await page.evaluate(() => {
    const listBounds = document.querySelector("#trip-list").getBoundingClientRect();
    const cardBounds = document
      .querySelector('.trip-card[data-leg-index="3"]')
      .getBoundingClientRect();
    return {
      listScrollTop: document.querySelector("#trip-list").scrollTop,
      fullyVisible:
        cardBounds.top >= listBounds.top - 1 && cardBounds.bottom <= listBounds.bottom + 1,
    };
  });
  expect(revealedCard.listScrollTop).toBeGreaterThan(0);
  expect(revealedCard.fullyVisible).toBe(true);
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

  await page.locator("#detail-content").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  expect(await page.locator("#detail-content").evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  await page.locator(".trip-card").nth(1).click();
  await expect(page.locator("body")).toHaveAttribute("data-selected-leg", "1");
  expect(await page.locator("#detail-content").evaluate((element) => element.scrollTop)).toBe(0);
  await expect(page.locator("#detail-heading")).toContainText("AY1415");
  await expect(page.locator("#detail-content")).toContainText("<img id=calendar-xss>");
  await expect(page.locator("#detail-content")).toContainText("Estimated arrival updated safely");
  await expect(page.locator("#detail-content")).not.toContainText("CAL-TRAVEL");
  for (const obsoleteLabel of [
    "Calendar id",
    "Live status",
    "Departure update",
    "Arrival update",
    "Departure delay",
    "Arrival delay",
  ]) {
    await expect(page.locator("#detail-content dt", {hasText: obsoleteLabel})).toHaveCount(0);
  }

  await page.locator('[data-projection="mercator"]').click();
  await expect(page.locator("body")).toHaveAttribute("data-projection", "mercator");
  await expect(page.locator("body")).toHaveAttribute("data-map-projection", "mercator");
  expect(apiCalls).toBe(1);
  await page.locator('[data-projection="globe"]').click();
  await expect(page.locator("body")).toHaveAttribute("data-projection", "globe");
  await expect(page.locator("body")).toHaveAttribute("data-map-projection", "globe");
  expect(apiCalls).toBe(1);

  await page.setViewportSize({width: 995, height: 554});
  const compactPanelLayout = await page.evaluate(() => {
    const panel = document.querySelector(".itinerary-panel");
    const detail = document.querySelector("#detail-panel").getBoundingClientRect();
    const warningList = document.querySelector("#warning-list");
    return {
      panelClientHeight: panel.clientHeight,
      panelScrollHeight: panel.scrollHeight,
      detailBottom: detail.bottom,
      viewportHeight: window.innerHeight,
      warningClientHeight: warningList.clientHeight,
      warningScrollHeight: warningList.scrollHeight,
    };
  });
  expect(compactPanelLayout.panelScrollHeight).toBeLessThanOrEqual(
    compactPanelLayout.panelClientHeight + 1,
  );
  expect(compactPanelLayout.detailBottom).toBeLessThanOrEqual(compactPanelLayout.viewportHeight);
  expect(compactPanelLayout.warningScrollHeight).toBeGreaterThan(
    compactPanelLayout.warningClientHeight,
  );

  await page.setViewportSize({width: 995, height: 480});
  const shortPanelLayout = await page.evaluate(() => {
    const panel = document.querySelector(".itinerary-panel");
    const panelBounds = panel.getBoundingClientRect();
    const selectorBounds = document.querySelector("#trip-selector").getBoundingClientRect();
    return {
      overflowY: getComputedStyle(panel).overflowY,
      panelClientHeight: panel.clientHeight,
      panelScrollHeight: panel.scrollHeight,
      panelTop: panelBounds.top,
      selectorTop: selectorBounds.top,
    };
  });
  expect(shortPanelLayout.overflowY).toBe("auto");
  expect(shortPanelLayout.selectorTop).toBeGreaterThanOrEqual(shortPanelLayout.panelTop);
  expect(shortPanelLayout.panelScrollHeight).toBeGreaterThan(shortPanelLayout.panelClientHeight);
  await page.locator(".itinerary-panel").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  const shortDetailBottom = await page
    .locator("#detail-panel")
    .evaluate((element) => element.getBoundingClientRect().bottom);
  expect(shortDetailBottom).toBeLessThanOrEqual(480);
  await page.locator(".itinerary-panel").evaluate((element) => {
    element.scrollTop = 0;
  });

  await page.setViewportSize({width: 500, height: 820});
  await expect(page.locator(".topbar")).toBeVisible();
  await expect(page.locator(".map-panel")).toBeVisible();
  await expect(page.locator(".itinerary-panel")).toBeVisible();
  const mobilePanelLayout = await page.evaluate(() => {
    const tripList = document.querySelector("#trip-list");
    const detailContent = document.querySelector("#detail-content");
    return {
      scrollY: window.scrollY,
      tripList: {clientHeight: tripList.clientHeight, scrollHeight: tripList.scrollHeight},
      detailContent: {
        clientHeight: detailContent.clientHeight,
        scrollHeight: detailContent.scrollHeight,
      },
    };
  });
  expect(mobilePanelLayout.scrollY).toBe(0);
  expect(mobilePanelLayout.tripList.clientHeight).toBe(mobilePanelLayout.tripList.scrollHeight);
  expect(mobilePanelLayout.detailContent.clientHeight).toBe(
    mobilePanelLayout.detailContent.scrollHeight,
  );
  await page.locator('.travel-marker.arrival[data-leg-index="3"]').click();
  await expect(page.locator("body")).toHaveAttribute("data-selected-leg", "3");
  const mobileReveal = await page.evaluate(() => {
    const cardBounds = document
      .querySelector('.trip-card[data-leg-index="3"]')
      .getBoundingClientRect();
    return {
      scrollY: window.scrollY,
      fullyVisible: cardBounds.top >= -1 && cardBounds.bottom <= window.innerHeight + 1,
    };
  });
  expect(mobileReveal.scrollY).toBeGreaterThan(0);
  expect(mobileReveal.fullyVisible).toBe(true);
  await page.setViewportSize({width: 1280, height: 720});

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
  expect(consoleErrors, `${consoleErrors.join("\n")}\n${serverStderr}`).toEqual([]);
});

test("consolidates booked and provider timing details", async ({page}) => {
  await page.route("**/assets/test-style.json", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        version: 8,
        sources: {},
        layers: [{id: "background", type: "background", paint: {"background-color": "#0b1317"}}],
      }),
    }),
  );
  await page.route("**/api/travel**", (route) =>
    route.fulfill({contentType: "application/json", body: JSON.stringify(detailPayload())}),
  );

  await page.goto(serverInfo.url, {waitUntil: "domcontentloaded"});
  const detail = page.locator("#detail-content");
  const timingGroup = (endpoint) => detail.locator(`.timing-group[data-endpoint="${endpoint}"]`);

  await expect(timingGroup("departure").locator(".timing-heading")).toHaveText(
    "Departure · Jul 15, 2026",
  );
  await expect(timingGroup("departure")).toContainText("Booked 16:00 (+02:00)");
  await expect(timingGroup("arrival")).toContainText("Booked 17:10 (+02:00)");
  await expect(detail).toContainText("Travel");
  await expect(detail).not.toContainText("CAL-TRAVEL");

  await page.locator('.trip-card[data-leg-index="1"]').click();
  await expect(timingGroup("departure")).toContainText("16:00 (+02:00) · On time");
  await expect(timingGroup("departure")).not.toContainText("Booked");
  await expect(timingGroup("arrival")).toContainText("17:10 (+02:00) · On time");
  await expect(detail).not.toContainText("Scheduled");

  await page.locator('.trip-card[data-leg-index="2"]').click();
  await expect(timingGroup("departure")).toContainText("Booked 16:00 (+02:00)");
  await expect(timingGroup("departure")).toContainText(
    "Updated 16:10 (+02:00) · 10 min late",
  );
  await expect(timingGroup("arrival")).toContainText("Booked 17:10 (+02:00)");
  await expect(timingGroup("arrival")).toContainText(
    "Updated 17:18 (+02:00) · 8 min late",
  );

  await page.locator('.trip-card[data-leg-index="3"]').click();
  await expect(timingGroup("departure")).toContainText(
    "Updated 16:12 (+02:00) · 12 min late",
  );
  await expect(timingGroup("departure")).not.toContainText("16:05 (+02:00)");
  await expect(timingGroup("arrival")).toContainText(
    "Updated 17:20 (+02:00) · 10 min late",
  );
  await expect(timingGroup("arrival")).not.toContainText("17:16 (+02:00)");

  await page.locator('.trip-card[data-leg-index="4"]').click();
  await expect(timingGroup("departure").locator(".timing-heading")).toHaveText(
    "Departure · Aug 1, 2026",
  );
  await expect(timingGroup("departure")).toContainText("Booked 16:35 (+03:00)");
  await expect(timingGroup("arrival").locator(".timing-heading")).toHaveText(
    "Arrival · Aug 2, 2026",
  );
  await expect(timingGroup("arrival")).toContainText("Booked 06:45 (+08:00)");

  await page.locator('.trip-card[data-leg-index="5"]').click();
  await expect(timingGroup("arrival").locator(".timing-heading")).toHaveText(
    "Arrival · Aug 1, 2026",
  );
  await expect(timingGroup("arrival")).toContainText("Booked 23:55 (+02:00)");
  await expect(timingGroup("arrival")).toContainText(
    "Updated Aug 2 · 00:15 (+02:00) · 20 min late",
  );

  await page.locator('.trip-card[data-leg-index="6"]').click();
  await expect(page.locator('.trip-card[data-leg-index="6"] .status-chip')).toHaveText("Diverted");
  await expect(detail.locator("dt", {hasText: "Diversion"})).toHaveCount(1);
  await expect(detail).toContainText("Rerouted via OSL due to weather");
  await expect(detail.locator("dt", {hasText: "Operational update"})).toHaveCount(0);

  await page.locator('.trip-card[data-leg-index="7"]').click();
  await expect(timingGroup("departure")).toContainText("Booked 16:00 (+02:00)");
  await expect(timingGroup("departure")).toContainText(
    "Updated 16:04 (+02:00) · 4 min late",
  );
  await expect(timingGroup("arrival")).toContainText("Updated 17:14 (+02:00) · 4 min late");

  await page.locator('.trip-card[data-leg-index="8"]').click();
  await expect(timingGroup("departure")).toContainText(
    "Updated 16:07 (+02:00) · 7 min late",
  );
  await expect(timingGroup("departure")).not.toContainText("16:04 (+02:00)");
  await expect(timingGroup("arrival")).toContainText("Updated 17:19 (+02:00) · 9 min late");
  await expect(timingGroup("arrival")).not.toContainText("17:14 (+02:00)");

  await page.setViewportSize({width: 1280, height: 720});
  const desktopTimingLayout = await page.evaluate(() => {
    const content = document.querySelector("#detail-content");
    const departure = document.querySelector('.timing-group[data-endpoint="departure"]');
    const arrival = document.querySelector('.timing-group[data-endpoint="arrival"]');
    const departureBounds = departure.getBoundingClientRect();
    const arrivalBounds = arrival.getBoundingClientRect();
    return {
      contentClientWidth: content.clientWidth,
      contentScrollWidth: content.scrollWidth,
      verticallyOrdered: departureBounds.bottom <= arrivalBounds.top,
      equalWidths: Math.abs(departureBounds.width - arrivalBounds.width) <= 1,
    };
  });
  expect(desktopTimingLayout.contentScrollWidth).toBeLessThanOrEqual(
    desktopTimingLayout.contentClientWidth + 1,
  );
  expect(desktopTimingLayout.verticallyOrdered).toBe(true);
  expect(desktopTimingLayout.equalWidths).toBe(true);

  await page.setViewportSize({width: 360, height: 740});
  const mobileTimingLayout = await page.evaluate(() => {
    const documentElement = document.documentElement;
    const content = document.querySelector("#detail-content");
    const contentBounds = content.getBoundingClientRect();
    const groups = [...document.querySelectorAll(".timing-group")].map((group) => {
      const bounds = group.getBoundingClientRect();
      return {left: bounds.left, right: bounds.right};
    });
    return {
      documentClientWidth: documentElement.clientWidth,
      documentScrollWidth: documentElement.scrollWidth,
      contentClientWidth: content.clientWidth,
      contentScrollWidth: content.scrollWidth,
      groupsInsideContent: groups.every(
        (bounds) =>
          bounds.left >= contentBounds.left - 1 && bounds.right <= contentBounds.right + 1,
      ),
    };
  });
  expect(mobileTimingLayout.documentScrollWidth).toBeLessThanOrEqual(
    mobileTimingLayout.documentClientWidth + 1,
  );
  expect(mobileTimingLayout.contentScrollWidth).toBeLessThanOrEqual(
    mobileTimingLayout.contentClientWidth + 1,
  );
  expect(mobileTimingLayout.groupsInsideContent).toBe(true);
  await expect(timingGroup("departure")).toContainText(
    "Updated 16:07 (+02:00) · 7 min late",
  );
  await expect(timingGroup("arrival")).toContainText("Updated 17:19 (+02:00) · 9 min late");

  for (const obsoleteLabel of [
    "Calendar id",
    "Live status",
    "Departure update",
    "Arrival update",
    "Departure delay",
    "Arrival delay",
  ]) {
    await expect(detail.locator("dt", {hasText: obsoleteLabel})).toHaveCount(0);
  }
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

async function textBounds(locator) {
  return locator.evaluate((element) => {
    const range = document.createRange();
    range.selectNodeContents(element);
    const bounds = range.getBoundingClientRect();
    return {x: bounds.x, width: bounds.width};
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
      {
        kind: "provider_warning",
        event_id: "EVENT-1",
        message: "Live departure gate is not available yet for this flight",
      },
      {
        kind: "provider_warning",
        event_id: "EVENT-2",
        message: "Live arrival gate is not available yet for this flight",
      },
      {
        kind: "provider_warning",
        event_id: "EVENT-3",
        message: "Airport metadata is incomplete and the route cannot be drawn",
      },
      {
        kind: "provider_warning",
        event_id: "EVENT-4",
        message: "Flight status is temporarily using stale cached provider data",
      },
    ],
    legs: [
      leg({
        eventId: "EVENT-1",
        flight: "D8 3229",
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

function detailPayload() {
  const from = airport("OSL", 60.1976, 11.1004, "Oslo Airport");
  const to = airport("CPH", 55.618, 12.6508, "Copenhagen Airport");
  const baseline = {
    from,
    to,
    departure: "2026-07-15T16:00:00+02:00",
    arrival: "2026-07-15T17:10:00+02:00",
    calendar: "Travel",
  };
  return {
    schema_version: 1,
    generated_at: "2026-07-14T08:00:00Z",
    range: {start: "2026-07-14", end: "2026-08-02", end_inclusive: true},
    calendar_ids: ["CAL-TRAVEL"],
    map: {projection: "globe", style_url: "/assets/test-style.json"},
    warnings: [],
    legs: [
      leg({eventId: "DETAIL-0", flight: "CAL1", ...baseline, live: null}),
      leg({
        eventId: "DETAIL-1",
        flight: "ON1",
        ...baseline,
        live: liveStatus("scheduled", "Scheduled", "fresh", {
          scheduled_departure: "2026-07-15T14:00:00Z",
          scheduled_arrival: "2026-07-15T15:10:00Z",
          estimated_departure: null,
          estimated_arrival: null,
          departure_delay_seconds: 0,
          arrival_delay_seconds: 0,
        }),
      }),
      leg({
        eventId: "DETAIL-2",
        flight: "EST2",
        ...baseline,
        live: liveStatus("scheduled", "Scheduled", "fresh", {
          estimated_departure: "2026-07-15T14:10:00Z",
          estimated_arrival: "2026-07-15T15:18:00Z",
          departure_delay_seconds: 600,
          arrival_delay_seconds: 480,
        }),
      }),
      leg({
        eventId: "DETAIL-3",
        flight: "ACT3",
        ...baseline,
        live: liveStatus("delayed", "Delayed", "fresh", {
          estimated_departure: "2026-07-15T14:05:00Z",
          actual_departure: "2026-07-15T14:12:00Z",
          estimated_arrival: "2026-07-15T15:16:00Z",
          actual_arrival: "2026-07-15T15:20:00Z",
          departure_delay_seconds: 720,
          arrival_delay_seconds: 600,
        }),
      }),
      leg({
        eventId: "DETAIL-4",
        flight: "NIGHT4",
        from,
        to,
        departure: "2026-08-01T16:35:00+03:00",
        arrival: "2026-08-02T06:45:00+08:00",
        calendar: "Travel",
        live: null,
      }),
      leg({
        eventId: "DETAIL-5",
        flight: "DATE5",
        from,
        to,
        departure: "2026-08-01T21:00:00+02:00",
        arrival: "2026-08-01T23:55:00+02:00",
        calendar: "Travel",
        live: liveStatus("delayed", "Delayed", "fresh", {
          estimated_departure: null,
          actual_departure: null,
          estimated_arrival: "2026-08-01T22:10:00Z",
          actual_arrival: "2026-08-01T22:15:00Z",
          departure_delay_seconds: 0,
          arrival_delay_seconds: 1200,
        }),
      }),
      leg({
        eventId: "DETAIL-6",
        flight: "DIV6",
        ...baseline,
        live: liveStatus("diverted", "Rerouted via OSL due to weather", "fresh", {
          diverted: true,
          estimated_departure: null,
          estimated_arrival: null,
          departure_delay_seconds: null,
          arrival_delay_seconds: null,
        }),
      }),
      leg({
        eventId: "DETAIL-7",
        flight: "SCH7",
        ...baseline,
        live: liveStatus("scheduled", "Scheduled", "fresh", {
          scheduled_departure: "2026-07-15T14:04:00Z",
          scheduled_arrival: "2026-07-15T15:14:00Z",
          estimated_departure: null,
          estimated_arrival: null,
          departure_delay_seconds: null,
          arrival_delay_seconds: null,
        }),
      }),
      leg({
        eventId: "DETAIL-8",
        flight: "EST8",
        ...baseline,
        live: liveStatus("scheduled", "Scheduled", "fresh", {
          scheduled_departure: "2026-07-15T14:04:00Z",
          scheduled_arrival: "2026-07-15T15:14:00Z",
          estimated_departure: "2026-07-15T14:07:00Z",
          estimated_arrival: "2026-07-15T15:19:00Z",
          departure_delay_seconds: null,
          arrival_delay_seconds: null,
        }),
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

function liveStatus(status, description, freshnessState, overrides = {}) {
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
    ...overrides,
  };
}
