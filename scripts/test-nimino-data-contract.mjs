#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-data/v1/contract.json", "utf8"),
);
const fixture = JSON.parse(
  readFileSync(
    "contracts/nimino-data/v1/fixtures/postgres-schema-0033.json",
    "utf8",
  ),
);
const nim = readFileSync(
  "nim/nimino_core/src/nimino_core/domain/data_contract.nim",
  "utf8",
);
const schema = readFileSync("schema/schema.sql", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function sorted(values) {
  return [...values].sort();
}

function nimRecordTypes(name) {
  const body = nim.match(
    new RegExp(`${name}\\* = \\[([\\s\\S]*?)\\n  \\]`),
  )?.[1];
  check(body, `missing Nim ${name}`);
  return [...body.matchAll(/"([a-z_]+)"/g)].map((match) => match[1]);
}

check(contract.protocol === "nimino.data", "wrong data protocol");
check(contract.version === 1, "wrong data contract version");
check(
  contract.transactionIntents["cache.replace"].replacementTarget ===
    "recordType is mandatory even when the replacement writes are empty" &&
    nim.includes("recordType*: string") &&
    nim.includes("dceRecordTypeRequired"),
  "cache replacement must retain an explicit target for empty clears",
);
check(fixture.schemaVersion === 33, "wrong PostgreSQL fixture version");
check(
  nim.includes(`DataProtocolName* = "${contract.protocol}"`) &&
    nim.includes(`DataProtocolVersion* = ${contract.version}`),
  "JSON and Nim protocol versions differ",
);

const taggedVariants = {
  transactionIntents: {
    "canonical.commit": "wiCanonicalCommit",
    "cache.replace": "wiCacheReplace",
    "log.append": "wiLogAppend",
  },
  queryIntents: {
    by_key: "qiByKey",
    page: "qiPage",
    changes: "qiChanges",
  },
};
for (const [section, variants] of Object.entries(taggedVariants)) {
  check(
    JSON.stringify(sorted(Object.keys(contract[section]))) ===
      JSON.stringify(sorted(Object.keys(variants))),
    `wrong ${section} variants`,
  );
  for (const nimVariant of Object.values(variants)) {
    check(nim.includes(nimVariant), `missing Nim variant: ${nimVariant}`);
  }
}

const classConstants = {
  canonical: "CanonicalRecordTypes",
  cache: "CacheRecordTypes",
  log: "LogRecordTypes",
};
const recordClasses = new Map();
for (const [dataClass, constant] of Object.entries(classConstants)) {
  const contractTypes = contract.classes[dataClass].recordTypes;
  check(
    JSON.stringify(sorted(contractTypes)) ===
      JSON.stringify(sorted(nimRecordTypes(constant))),
    `${dataClass} record types differ between JSON and Nim`,
  );
  for (const recordType of contractTypes) {
    check(
      !recordClasses.has(recordType),
      `duplicate record type: ${recordType}`,
    );
    recordClasses.set(recordType, dataClass);
  }
}

const fixtureTables = new Set();
for (const entry of fixture.tables) {
  check(
    !fixtureTables.has(entry.table),
    `duplicate fixture table: ${entry.table}`,
  );
  fixtureTables.add(entry.table);
  check(
    recordClasses.get(entry.recordType) === entry.class,
    `invalid ${entry.class} mapping for ${entry.table}`,
  );
}
check(
  fixtureTables.size === 36,
  "active logical table fixture must contain 36 tables",
);

const activeSchemaTables = [...schema.matchAll(/^CREATE TABLE ([a-z0-9_]+)/gm)]
  .map((match) => match[1])
  .filter(
    (name) =>
      name !== "_operator_global_tables" &&
      !name.startsWith("events_p") &&
      !name.startsWith("delivery_log_p"),
  );
check(
  JSON.stringify(sorted(activeSchemaTables)) ===
    JSON.stringify(sorted(fixtureTables)),
  "PostgreSQL logical tables differ from the v1 compatibility fixture",
);

for (const entry of fixture.embedded) {
  check(
    recordClasses.get(entry.recordType) === entry.class,
    `invalid embedded mapping for ${entry.source}`,
  );
}
for (const projection of contract.projectionRebuild.projections) {
  check(
    recordClasses.get(projection.rowType) === "cache",
    `projection ${projection.name} must emit cache rows`,
  );
  const nimVariant = `pk${projection.name[0].toUpperCase()}${projection.name.slice(1)}`;
  check(
    nim.includes(nimVariant),
    `missing Nim projection variant: ${nimVariant}`,
  );
}

console.log("Nimino data contract verified");
