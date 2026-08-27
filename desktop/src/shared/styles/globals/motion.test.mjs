import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const motionCss = readFileSync(
  new URL("./motion.css", import.meta.url),
  "utf8",
);
const sourceRoot = fileURLToPath(new URL("../../../", import.meta.url));
const landingBees = readFileSync(
  new URL("../../../features/onboarding/ui/LandingBees.tsx", import.meta.url),
  "utf8",
);
const searchPrompt = readFileSync(
  new URL(
    "../../../features/search/ui/SearchPromptPlaceholder.tsx",
    import.meta.url,
  ),
  "utf8",
);
const welcomeComposerBanner = readFileSync(
  new URL(
    "../../../features/channels/ui/WelcomeComposerBanner.tsx",
    import.meta.url,
  ),
  "utf8",
);

test("shared motion budget is at most 200ms", () => {
  const durations = [
    ...motionCss.matchAll(/--motion-duration-[^:]+:\s*(\d+)ms/g),
  ];
  assert.ok(durations.length > 0);
  assert.deepEqual(
    durations.filter(([, milliseconds]) => Number(milliseconds) > 200),
    [],
  );
});

test("CSS animation and transition durations stay within 200ms", () => {
  const violations = readdirSync(sourceRoot, { recursive: true })
    .filter((path) => path.endsWith(".css"))
    .flatMap((path) => {
      const source = readFileSync(resolve(sourceRoot, path), "utf8");
      return [
        ...source.matchAll(
          /(?:animation(?:-duration)?|transition(?:-duration)?):[^;]*;/g,
        ),
      ]
        .filter(([declaration]) => !/\binfinite\b/.test(declaration))
        .filter(([declaration]) =>
          [...declaration.matchAll(/(\d+(?:\.\d+)?)(ms|s)/g)].some(
            ([, duration, unit]) =>
              Number(duration) * (unit === "s" ? 1000 : 1) > 200,
          ),
        )
        .map(
          ([declaration]) =>
            `${relative(sourceRoot, resolve(sourceRoot, path))}:${declaration.trim()}`,
        );
    });
  assert.deepEqual(violations, []);
});

test("motion component durations stay within 200ms", () => {
  const violations = readdirSync(sourceRoot, { recursive: true })
    .filter((path) => /\.tsx?$/.test(path))
    .flatMap((path) => {
      const source = readFileSync(resolve(sourceRoot, path), "utf8");
      if (!source.includes('from "motion/react"')) {
        return [];
      }
      return [...source.matchAll(/duration:\s*(\d+(?:\.\d+)?)/g)]
        .filter(([, seconds]) => Number(seconds) > 0.2)
        .map(
          ([duration]) =>
            `${relative(sourceRoot, resolve(sourceRoot, path))}:${duration}`,
        );
    });
  assert.deepEqual(violations, []);
});

test("Web Animation durations stay within 200ms", () => {
  const violations = readdirSync(sourceRoot, { recursive: true })
    .filter((path) => /\.tsx?$/.test(path))
    .flatMap((path) => {
      const source = readFileSync(resolve(sourceRoot, path), "utf8");
      if (!source.includes(".animate(")) {
        return [];
      }
      return [
        ...source.matchAll(/(?:[A-Z_]*DURATION_MS\s*=\s*|duration:\s*)(\d+)/g),
      ]
        .filter(([, milliseconds]) => Number(milliseconds) > 200)
        .map(
          ([duration]) =>
            `${relative(sourceRoot, resolve(sourceRoot, path))}:${duration}`,
        );
    });
  assert.deepEqual(violations, []);
});

test("reduced motion settles every animation and transition immediately", () => {
  assert.match(
    motionCss,
    /@media \(prefers-reduced-motion: reduce\)[\s\S]*\*,[\s\S]*\*::before,[\s\S]*\*::after[\s\S]*animation-duration:\s*0ms !important;[\s\S]*animation-iteration-count:\s*1 !important;[\s\S]*transition-duration:\s*0ms !important;/,
  );
});

test("continuous CSS motion is reserved for the functional spinner", () => {
  const infiniteAnimations = readdirSync(sourceRoot, { recursive: true })
    .filter((path) => path.endsWith(".css"))
    .flatMap((path) =>
      readFileSync(resolve(sourceRoot, path), "utf8")
        .split("\n")
        .filter((line) => /animation:.*\binfinite\b/.test(line.trim()))
        .map(
          (line) =>
            `${relative(sourceRoot, resolve(sourceRoot, path))}:${line.trim()}`,
        ),
    );
  assert.deepEqual(infiniteAnimations, [
    "shared/styles/globals/animations.css:animation: sprout-arc-spinner-spin 500ms linear infinite;",
  ]);
});

test("component motion cannot repeat forever", () => {
  const continuousMotion = readdirSync(sourceRoot, { recursive: true })
    .filter((path) => /\.tsx?$/.test(path))
    .flatMap((path) =>
      readFileSync(resolve(sourceRoot, path), "utf8")
        .split("\n")
        .filter((line) =>
          /repeat:\s*(?:Number\.POSITIVE_INFINITY|Infinity)|repeatCount=.*indefinite/.test(
            line,
          ),
        )
        .map(
          (line) =>
            `${relative(sourceRoot, resolve(sourceRoot, path))}:${line.trim()}`,
        ),
    );
  assert.deepEqual(continuousMotion, []);
});

test("decorative surfaces are static", () => {
  assert.doesNotMatch(landingBees, /requestAnimationFrame|mousemove|mouseout/);
  assert.doesNotMatch(searchPrompt, /setInterval|motion\/react/);
  assert.doesNotMatch(
    welcomeComposerBanner,
    /setInterval|welcome-composer-persona-character/,
  );
  assert.equal(
    existsSync(
      new URL("../../../shared/ui/buzz-logo/FlappingBee.tsx", import.meta.url),
    ),
    false,
  );
});
