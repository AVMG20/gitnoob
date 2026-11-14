import { describe, expect, it } from "vitest";
import { fileIcon, folderIcon, iconFor } from "~/composables/fileIcons";

const glyph = (path: string) => fileIcon(path).glyph;
const tone = (path: string) => fileIcon(path).tone;

describe("file icons", () => {
  it("goes by the whole name before the extension", () => {
    // package.json is a JSON file, but it is the npm one.
    expect(tone("package.json")).toBe("npm");
    expect(tone("web/package.json")).toBe("npm");
    expect(tone("tsconfig.json")).toBe("json");
    expect(tone("composer.json")).toBe("composer");
    expect(tone("composer.lock")).toBe("lock");
    expect(fileIcon("src-tauri/Cargo.toml").glyph).toBe("box");
    expect(tone("pyproject.toml")).toBe("toml");
    expect(tone("Dockerfile")).toBe("docker");
    expect(tone("docker-compose.prod.yml")).toBe("docker");
    expect(tone("Makefile")).toBe("make");
    expect(tone("artisan")).toBe("laravel");
  });

  it("reads extensions without caring about case", () => {
    expect(tone("src/Main.TS")).toBe("ts");
    expect(tone("LOGO.PNG")).toBe("image");
    expect(tone("README.MD")).toBe("readme");
    expect(fileIcon("App.Vue").glyph).toBe("vue");
  });

  it("takes a prefix only as a word of its own", () => {
    // Code that happens to start like a README or a licence is code.
    expect(tone("src/ReadmeParser.ts")).toBe("ts");
    expect(tone("licenses.json")).toBe(tone("data.json"));
    expect(tone("copyingutil.js")).toBe(tone("main.js"));
    expect(tone("README")).toBe("readme");
    expect(tone("readme-dev.md")).toBe("readme");
    expect(tone("LICENSE.txt")).toBe(tone("LICENSE"));
    expect(tone(".env.local")).toBe(tone(".env"));
    expect(tone("docker-compose.prod.yml")).toBe(tone("compose.yaml"));
    // A Blade view is one whatever it is called.
    expect(tone("readme.blade.php")).toBe("blade");
  });

  it("knows the shell's dotfiles", () => {
    expect(tone(".zshrc")).toBe("shell");
    expect(tone(".bashrc")).toBe("shell");
    expect(tone(".npmrc")).toBe("npm");
  });

  it("tells a Blade template from plain PHP", () => {
    expect(tone("resources/views/home.blade.php")).toBe("blade");
    expect(tone("app/Models/User.php")).toBe("php");
    expect(fileIcon("User.php").text).toBe("P");
  });

  it("knows the dotfiles", () => {
    expect(tone(".gitignore")).toBe("git");
    expect(tone(".env")).toBe("env");
    expect(tone(".env.local")).toBe("env");
    expect(tone(".env.example")).toBe("env");
    expect(tone(".editorconfig")).toBe("config");
    // A dotfile nobody listed is a plain file, not "an extension of nothing".
    expect(tone(".prettierrc")).toBe("plain");
  });

  it("gives a plain file to no extension and to anything unknown", () => {
    expect(glyph("bin/console")).toBe("file");
    expect(glyph("notes.unknownext")).toBe("file");
    expect(glyph("archive.")).toBe("file");
    expect(glyph("LICENSE")).toBe("scale");
    expect(glyph("README")).toBe("book");
  });

  it("covers the languages asked for", () => {
    const tones: Record<string, string> = {
      "a.js": "js",
      "a.mjs": "js",
      "a.cjs": "js",
      "a.jsx": "react",
      "a.tsx": "react",
      "a.go": "go",
      "a.rs": "rust",
      "a.py": "python",
      "a.rb": "ruby",
      "a.java": "java",
      "a.kt": "kotlin",
      "a.swift": "swift",
      "a.c": "c",
      "a.h": "c",
      "a.cpp": "cpp",
      "a.cs": "csharp",
      "a.sh": "shell",
      "a.zsh": "shell",
      "a.bash": "shell",
      "a.nu": "shell",
      "a.yaml": "yaml",
      "a.yml": "yaml",
      "a.xml": "xml",
      "a.html": "html",
      "a.css": "css",
      "a.scss": "scss",
      "a.sass": "scss",
      "a.less": "less",
      "a.sql": "sql",
      "a.svg": "svg",
      "a.jpg": "image",
      "a.gif": "image",
      "a.webp": "image",
      "a.ico": "image",
      "yarn.lock": "lock",
      "a.ini": "config",
      "a.conf": "config",
      "a.txt": "text",
      "a.csv": "csv",
      "a.pdf": "pdf",
      "a.zip": "archive",
      "a.tar": "archive",
      "a.gz": "archive",
      "a.md": "markdown",
    };
    for (const [name, expected] of Object.entries(tones))
      expect([name, tone(name)]).toEqual([name, expected]);
  });

  it("hands back the same object for the same name, so a list allocates nothing", () => {
    expect(fileIcon("a/index.ts")).toBe(fileIcon("b/index.ts"));
    expect(folderIcon("x/src", true)).toBe(folderIcon("y/src", true));
  });

  it("tints the folders people look for, faintly, and opens them", () => {
    expect(folderIcon("vendor").tone).toBe("folder-composer");
    expect(folderIcon("node_modules").tone).toBe("folder-npm");
    expect(folderIcon(".github").tone).toBe("folder-github");
    expect(folderIcon("app/Billing").tone).toBe("folder");
    expect(folderIcon("app/Billing", true).glyph).toBe("folder-open");
    expect(iconFor("src", true).glyph).toBe("folder");
    expect(iconFor("src", false).glyph).toBe("file");
  });
});
