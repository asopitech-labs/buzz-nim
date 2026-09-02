import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

import {
  ComposerMessageLinkNode,
  registerComposerMessageLinkMarkdownIt,
  resolveComposerMessageLinkAttributes,
} from "./composerMessageLinkNode.ts";

const requireFromTiptap = createRequire(import.meta.resolve("tiptap-markdown"));
const MarkdownIt = requireFromTiptap("markdown-it");

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const MESSAGE_ID = "root-event";
const HREF = `nimino://message?channel=${CHANNEL_ID}&id=${MESSAGE_ID}`;
const CHANNEL_HREF = `nimino://channel/${CHANNEL_ID}`;
const CHANNEL_MESSAGE_ID = "a".repeat(64);
const CHANNEL_MESSAGE_HREF = `nimino://channel/${CHANNEL_ID}/${CHANNEL_MESSAGE_ID}`;
const OWNER = "a".repeat(64);
const REPO_HREF = `nimino://repo?owner=${OWNER}&d=nimino-world`;
const ISSUE_ID = "b".repeat(64);
const ISSUE_HREF = `nimino://issue?id=${ISSUE_ID}&owner=${OWNER}&d=nimino-world`;
const PR_ID = "c".repeat(64);
const PR_HREF = `nimino://pr?id=${PR_ID}&owner=${OWNER}&d=nimino-world`;

test("resolves a composer preview and canonicalizes the underlying href", () => {
  assert.deepEqual(
    resolveComposerMessageLinkAttributes(HREF, (channelId) =>
      channelId === CHANNEL_ID ? "general" : undefined,
    ),
    { channelName: "general", href: HREF },
  );
});

test("rejects the legacy runtime scheme", () => {
  assert.equal(
    resolveComposerMessageLinkAttributes(
      HREF.replace("nimino://", ["bu", "zz://"].join("")),
      () => "general",
    ),
    null,
  );
});

test("rejects malformed message links", () => {
  assert.equal(
    resolveComposerMessageLinkAttributes(
      `nimino://message?channel=${CHANNEL_ID}`,
      () => "general",
    ),
    null,
  );
});

test("resolves channel and entity links as composer chips", () => {
  assert.deepEqual(
    resolveComposerMessageLinkAttributes(CHANNEL_HREF, (channelId) =>
      channelId === CHANNEL_ID ? "general" : undefined,
    ),
    { channelName: "general", href: CHANNEL_HREF },
  );
  assert.deepEqual(
    resolveComposerMessageLinkAttributes(CHANNEL_MESSAGE_HREF, (channelId) =>
      channelId === CHANNEL_ID ? "general" : undefined,
    ),
    {
      channelName: "general",
      href: `nimino://message?channel=${CHANNEL_ID}&id=${CHANNEL_MESSAGE_ID}`,
    },
  );
  assert.deepEqual(
    resolveComposerMessageLinkAttributes(REPO_HREF, () => undefined),
    { channelName: "", href: REPO_HREF },
  );
  assert.deepEqual(
    resolveComposerMessageLinkAttributes(ISSUE_HREF, () => undefined),
    { channelName: "", href: ISSUE_HREF },
  );
});

function captureMarkdownRule() {
  let capturedAnchor = null;
  let capturedRule = null;
  const md = {
    renderer: { rules: {} },
    inline: {
      ruler: {
        before(anchor, _name, rule) {
          capturedAnchor = anchor;
          capturedRule = rule;
        },
      },
    },
    utils: {
      escapeHtml: (value) => value.replaceAll("&", "&amp;"),
    },
  };
  registerComposerMessageLinkMarkdownIt(md, {
    resolveChannelName: (channelId) =>
      channelId === CHANNEL_ID ? "general" : undefined,
  });
  return { anchor: capturedAnchor, md, rule: capturedRule };
}

test("markdown parsing materializes a bare message link in composer content", () => {
  const { anchor, rule } = captureMarkdownRule();
  assert.equal(anchor, "text");
  let token = null;
  const state = {
    src: `See ${HREF}.`,
    pos: 4,
    push: () => {
      token = { meta: null };
      return token;
    },
  };

  assert.equal(rule(state, false), true);
  assert.equal(state.pos, 4 + HREF.length);
  assert.deepEqual(token.meta, { channelName: "general", href: HREF });
});

test("real markdown-it parsing materializes a restored message link", () => {
  const md = new MarkdownIt();
  registerComposerMessageLinkMarkdownIt(md, {
    resolveChannelName: (channelId) =>
      channelId === CHANNEL_ID ? "general" : undefined,
  });

  const html = md.renderInline(`See ${HREF}.`);
  assert.match(html, /See <span data-composer-nimino-link=""/);
  assert.match(html, /data-channel-name="general"/);
  assert.match(html, /data-href="nimino:\/\/message\?channel=.*&amp;id=/);
});

test("real markdown-it parsing materializes mixed Nimino permalink chips", () => {
  const md = new MarkdownIt();
  registerComposerMessageLinkMarkdownIt(md, {
    resolveChannelName: (channelId) =>
      channelId === CHANNEL_ID ? "general" : undefined,
  });

  const html = md.renderInline(`${HREF} ${CHANNEL_HREF} ${REPO_HREF}`);
  assert.equal((html.match(/data-composer-nimino-link=""/g) ?? []).length, 3);
  assert.match(html, /data-href="nimino:\/\/channel\/9a1657ac/);
  assert.match(
    html,
    /data-href="nimino:\/\/repo\?owner=a{64}&amp;d=nimino-world/,
  );
});

test("real markdown-it parsing preserves underscores in restored entity links", () => {
  const md = new MarkdownIt();
  registerComposerMessageLinkMarkdownIt(md, {
    resolveChannelName: () => undefined,
  });
  const href = `nimino://repo?owner=${OWNER}&d=my_repo`;

  const html = md.renderInline(href);

  assert.equal((html.match(/data-composer-nimino-link=""/g) ?? []).length, 1);
  assert.match(html, /data-href="nimino:\/\/repo\?owner=a{64}&amp;d=my_repo"/);
  assert.doesNotMatch(html, /<\/span>_repo/);
});

test("markdown parsing resumes after markdown-it consumes the nimino prefix", () => {
  const { rule } = captureMarkdownRule();
  let token = null;
  const state = {
    pending: "See nimino",
    src: `See ${HREF}`,
    pos: "See nimino".length,
    push: () => {
      token = { meta: null };
      return token;
    },
  };

  assert.equal(rule(state, false), true);
  assert.equal(state.pending, "See ");
  assert.equal(state.pos, state.src.length);
  assert.deepEqual(token.meta, { channelName: "general", href: HREF });
});

test("markdown parsing stops message links before emphasis delimiters", () => {
  const { rule } = captureMarkdownRule();
  let token = null;
  const state = {
    src: `${HREF}*`,
    pos: 0,
    push: () => {
      token = { meta: null };
      return token;
    },
  };

  assert.equal(rule(state, false), true);
  assert.equal(state.pos, HREF.length);
  assert.deepEqual(token.meta, { channelName: "general", href: HREF });
});

test("composer node uses the sent-message chip presentation", () => {
  const node = {
    attrs: { channelName: "general", href: HREF },
  };
  const rendered = globalThis.structuredClone(
    // TipTap invokes renderHTML with the extension instance as `this`.
    // Exercise the production renderer directly so the composer and message
    // list cannot silently drift back to separate visual languages.
    ComposerMessageLinkNode.config.renderHTML.call(
      { options: { resolveChannelName: () => "general" } },
      { HTMLAttributes: {}, node },
    ),
  );

  assert.equal(rendered[0], "span");
  assert.match(rendered[1].class, /mention-chip/);
  assert.match(rendered[1].class, /inline-chip-with-icon/);
  assert.match(rendered[1].class, /inline-chip-icon-message/);
  assert.equal(rendered[1]["data-nimino-link"], "");
  // Channel label only — no event hash, so the chip does not change width when
  // the draft is sent and the rendered chip resolves its metadata.
  assert.equal(rendered[2], "general");
});

test("composer node renders channel and entity chip presentations", () => {
  const render = (href) =>
    globalThis.structuredClone(
      ComposerMessageLinkNode.config.renderHTML.call(
        { options: { resolveChannelName: () => "general" } },
        {
          HTMLAttributes: {},
          node: { attrs: { channelName: "general", href } },
        },
      ),
    );

  const channel = render(CHANNEL_HREF);
  assert.equal(channel[1]["data-channel-deep-link"], "");
  assert.match(channel[1].class, /inline-chip-icon-channel/);
  assert.equal(channel[2], "general");

  const repo = render(REPO_HREF);
  assert.equal(repo[1]["data-nimino-link-kind"], "repo");
  assert.match(repo[1].class, /inline-chip-icon-repo/);
  assert.equal(repo[2], "nimino-world");

  const issue = render(ISSUE_HREF);
  assert.equal(issue[1]["data-nimino-link-kind"], "issue");
  assert.match(issue[1].class, /inline-chip-icon-issue/);
  // Repository name only — the rendered chip never widens into the issue
  // title, so the composer must not widen into the event hash either.
  assert.equal(issue[2], "nimino-world");

  const pullRequest = render(PR_HREF);
  assert.equal(pullRequest[1]["data-nimino-link-kind"], "pr");
  assert.match(pullRequest[1].class, /inline-chip-icon-pr/);
  assert.equal(pullRequest[2], "nimino-world");
});

test("markdown rendering stores identity in attributes, not visible id text", () => {
  const { md } = captureMarkdownRule();
  const render = md.renderer.rules.nimino_composer_message_link;
  const html = render([{ meta: { channelName: "general", href: HREF } }], 0);

  assert.match(html, /data-composer-nimino-link=""/);
  assert.match(html, /data-channel-name="general"/);
  assert.match(html, /data-href="nimino:\/\/message\?channel=.*&amp;id=/);
  assert.doesNotMatch(html, />[^<]*root-event/);
});
