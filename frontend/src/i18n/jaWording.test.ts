import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { literalTranslations } from "./literalTranslations";
import ja from "./resources/ja";

/**
 * Words the Japanese text avoids, each with what to write instead. Only words a search can find without reading the
 * sentence are here. Words that are wrong in one sense and right in another (入れる／消す for installing, 版, 確認,
 * 囲む, メイン, 答える) are left to review. So are アップロード and バックエンド: the editor's table still has
 * them in rows the screen no longer shows and in the keypoint debug panel.
 */
const AVOIDED: { word: RegExp; use: string }[] = [
  { word: /返事/u, use: "応答" },
  { word: /確かめ/u, use: "確認してください" },
  { word: /サブスク/u, use: "有料プラン" },
  { word: /エディタ(?!ー)/u, use: "エディター" },
  { word: /パラメータ(?!ー)/u, use: "パラメーター" },
  { word: /フォルダ(?!ー)/u, use: "フォルダー" },
  // Said of a thing, 使える／使えない sound like "capable／useless". 使えば (if you use) is fine.
  { word: /使え(?!ば)/u, use: "使用できます・使用できません・対応していません" },
  { word: /止めました/u, use: "停止しました" },
  { word: /無くなりました/u, use: "廃止されました" },
  { word: /このマシン|このコンピューター/u, use: "この PC" },
  { word: /部分生成/u, use: "部分再生成" },
  { word: /矩形/u, use: "四角形" },
  { word: /経路|つなぎ先|接続先/u, use: "API キー方式・AI サービス（画面で選ぶ名前）" },
  { word: /取ってきて/u, use: "ダウンロードして" },
  { word: /作った画像/u, use: "生成した画像" },
];

const JAPANESE = "\\p{Script=Hiragana}\\p{Script=Katakana}\\p{Script=Han}ー";
/** A Japanese character right next to a Latin letter or a digit, with no space between (「API キー」「1 回」). */
const UNSPACED = new RegExp(`[${JAPANESE}][A-Za-z0-9]|[A-Za-z0-9][${JAPANESE}]`, "gu");

function strings(value: unknown, path: string): [where: string, text: string][] {
  if (typeof value === "string") return [[path, value]];
  if (value !== null && typeof value === "object") {
    return Object.entries(value).flatMap(([key, child]) => strings(child, `${path}.${key}`));
  }
  return [];
}

/** Every Japanese text the user reads: the screen's resources, the editor's table (its values) and README.ja.md. */
function japaneseTexts(): [where: string, text: string][] {
  const readme = readFileSync(join(import.meta.dirname, "../../../README.ja.md"), "utf8");
  return [
    ...strings(ja, "ja"),
    ...Object.entries(literalTranslations.ja).map(([key, text]): [string, string] => [`literal「${key}」`, text]),
    ...readme.split("\n").map((line, index): [string, string] => [`README.ja.md:${index + 1}`, line]),
  ];
}

describe("Japanese wording", () => {
  it("reads every source", () => {
    const wheres = japaneseTexts().map(([where]) => where);
    expect(wheres.filter((where) => where.startsWith("ja.")).length).toBeGreaterThan(300);
    expect(wheres.filter((where) => where.startsWith("literal")).length).toBeGreaterThan(300);
    expect(wheres.filter((where) => where.startsWith("README")).length).toBeGreaterThan(50);
  });

  it("uses none of the avoided words", () => {
    const found = japaneseTexts().flatMap(([where, text]) =>
      AVOIDED.flatMap(({ word, use }) => {
        const hit = text.match(word)?.[0];
        return hit ? [`${where}: 「${hit}」→「${use}」: ${text}`] : [];
      }),
    );
    expect(found).toEqual([]);
  });

  it("puts a space between Japanese and Latin letters or digits", () => {
    const found = japaneseTexts().flatMap(([where, text]) => {
      const hits = text.match(UNSPACED);
      return hits ? [`${where}: ${hits.join(" ")}: ${text}`] : [];
    });
    expect(found).toEqual([]);
  });

  it("finds what it looks for", () => {
    const flagged = (text: string) => AVOIDED.some(({ word }) => word.test(text));
    expect(flagged("返事が空でした")).toBe(true);
    expect(flagged("この画像は使えません")).toBe(true);
    expect(flagged("エディタを開く")).toBe(true);
    expect(flagged("エディターを開く")).toBe(false);
    expect(flagged("「OpenAI 互換 API」を選んだときだけ使えます")).toBe(true);
    expect(flagged("「OpenAI 互換 API」を選んだときだけ使用できます")).toBe(false);
    expect(flagged("この形式を使えば開けます")).toBe(false);
    expect("1回".match(UNSPACED)).toEqual(["1回"]);
    expect("API キー".match(UNSPACED)).toBeNull();
  });
});
