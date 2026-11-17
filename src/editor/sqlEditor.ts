// The CodeMirror 6 editor. This module (and everything it imports) is loaded
// lazily, after the window is usable, so it never counts against cold start
// or the initial bundle.

import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { PostgreSQL, SQLite, StandardSQL, sql, type SQLDialect } from '@codemirror/lang-sql';
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  HighlightStyle,
  indentOnInput,
  indentUnit,
  syntaxHighlighting,
} from '@codemirror/language';
import { highlightSelectionMatches, search, searchKeymap } from '@codemirror/search';
import {
  Compartment,
  EditorState,
  StateEffect,
  StateField,
  type Extension,
} from '@codemirror/state';
import {
  Decoration,
  type DecorationSet,
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder,
  rectangularSelection,
  crosshairCursor,
} from '@codemirror/view';
import { tags as t } from '@lezer/highlight';

export type Dialect = 'postgres' | 'clickhouse' | 'sqlite' | null;

const DIALECTS: Record<string, SQLDialect> = {
  postgres: PostgreSQL,
  sqlite: SQLite,
  // A ClickHouse dialect arrives with autocomplete (M2).
  clickhouse: StandardSQL,
};

// The database's error position, shown as a squiggle.
const setErrorMark = StateEffect.define<{ from: number; to: number } | null>();
const errorMark = Decoration.mark({ class: 'cm-sql-error' });
const errorField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(marks, transaction) {
    // Any edit clears the marker: it described the text as it was when run.
    if (transaction.docChanged) return Decoration.none;
    for (const effect of transaction.effects) {
      if (effect.is(setErrorMark)) {
        return effect.value
          ? Decoration.set([errorMark.range(effect.value.from, effect.value.to)])
          : Decoration.none;
      }
    }
    return marks;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export interface EditorOptions {
  tabSize: number;
  indentWithTabs: boolean;
  wordWrap: boolean;
  lineNumbers: boolean;
  autoCloseBrackets: boolean;
  readOnly?: boolean;
  dialect?: Dialect;
}

// Colors come from CSS variables, so light/dark switches need no re-theme.
const theme = EditorView.theme({
  '&': { height: '100%', color: 'var(--text)', backgroundColor: 'var(--bg)' },
  '.cm-scroller': {
    fontFamily: 'var(--editor-font)',
    fontSize: 'var(--editor-font-size)',
    lineHeight: '1.55',
  },
  '.cm-content': { caretColor: 'var(--text)', padding: '8px 0' },
  '&.cm-focused': { outline: 'none' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground':
    { backgroundColor: 'var(--bg-active)' },
  '.cm-gutters': {
    backgroundColor: 'var(--bg)',
    color: 'var(--text-faint)',
    border: 'none',
  },
  '.cm-lineNumbers .cm-gutterElement': { padding: '0 10px 0 14px' },
  '.cm-activeLine': { backgroundColor: 'var(--bg-hover)' },
  '.cm-activeLineGutter': { backgroundColor: 'var(--bg-hover)', color: 'var(--text-muted)' },
  '.cm-matchingBracket': {
    backgroundColor: 'var(--bg-active)',
    outline: '1px solid var(--border-strong)',
  },
  '.cm-selectionMatch': { backgroundColor: 'var(--bg-inset)' },
  '.cm-foldPlaceholder': {
    backgroundColor: 'var(--bg-inset)',
    border: 'none',
    color: 'var(--text-muted)',
  },
  '.cm-placeholder': { color: 'var(--text-faint)' },
  '.cm-panels': {
    backgroundColor: 'var(--bg-subtle)',
    color: 'var(--text)',
    borderTop: '1px solid var(--border)',
  },
  '.cm-panel.cm-search': { padding: '6px 8px' },
  '.cm-panel.cm-search input, .cm-panel.cm-search button': {
    font: 'inherit',
    color: 'var(--text)',
    background: 'var(--bg)',
    border: '1px solid var(--border-strong)',
    borderRadius: '4px',
  },
  '.cm-sql-error': {
    textDecoration: 'underline wavy var(--danger)',
    textDecorationSkipInk: 'none',
    textUnderlineOffset: '3px',
  },
  '.cm-searchMatch': { backgroundColor: 'rgba(232, 163, 61, 0.35)' },
  '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'rgba(232, 163, 61, 0.7)' },
});

const highlight = HighlightStyle.define([
  {
    tag: [t.keyword, t.operatorKeyword, t.modifier],
    color: 'var(--syn-keyword)',
    fontWeight: '600',
  },
  { tag: [t.string, t.special(t.string)], color: 'var(--syn-string)' },
  { tag: [t.number, t.bool, t.null], color: 'var(--syn-number)' },
  { tag: [t.lineComment, t.blockComment], color: 'var(--syn-comment)', fontStyle: 'italic' },
  { tag: [t.typeName, t.standard(t.name)], color: 'var(--syn-type)' },
  { tag: [t.operator, t.punctuation, t.paren, t.bracket], color: 'var(--syn-operator)' },
  { tag: t.name, color: 'var(--syn-name)' },
]);

export class SqlEditor {
  readonly view: EditorView;
  #states = new Map<number, EditorState>();
  #current: number | null = null;
  #options: EditorOptions;
  #indent = new Compartment();
  #wrap = new Compartment();
  #gutter = new Compartment();
  #brackets = new Compartment();
  #language = new Compartment();
  #readOnly = new Compartment();
  #onChange: (tabId: number, text: string) => void;

  constructor(
    parent: HTMLElement,
    options: EditorOptions,
    onChange: (tabId: number, text: string) => void,
  ) {
    this.#options = options;
    this.#onChange = onChange;
    this.view = new EditorView({ parent, state: this.#newState('') });
  }

  /** Shows a tab's document, creating its editor state on first use. */
  show(tabId: number, text: string): void {
    if (this.#current === tabId) return;
    if (this.#current !== null) this.#states.set(this.#current, this.view.state);
    this.#current = tabId;
    this.view.setState(this.#states.get(tabId) ?? this.#newState(text));
    // A stored state may predate a settings change.
    this.#reconfigure();
  }

  /** Drops a closed tab's undo history and document. */
  forget(tabId: number): void {
    this.#states.delete(tabId);
    if (this.#current === tabId) this.#current = null;
  }

  configure(options: EditorOptions): void {
    this.#options = options;
    this.#reconfigure();
  }

  /** The main selection (from === to for a bare cursor), UTF-16 offsets. */
  selection(): { from: number; to: number } {
    const { from, to } = this.view.state.selection.main;
    return { from, to };
  }

  /** Underlines the token at `offset` and scrolls it into view; null clears. */
  markError(offset: number | null): void {
    if (offset === null) {
      this.view.dispatch({ effects: setErrorMark.of(null) });
      return;
    }
    const doc = this.view.state.doc;
    const from = Math.min(Math.max(offset, 0), doc.length);
    // Extend to the end of the word, or one character if there is no word.
    const rest = doc.sliceString(from, Math.min(doc.length, from + 200));
    const word = /^[\w$"`.]+/.exec(rest)?.[0].length ?? 0;
    let to = Math.min(doc.length, from + Math.max(word, 1));
    let start = from;
    if (start === to && start > 0) start -= 1; // error at end of input
    if (start === to) return;
    to = Math.max(to, start + 1);
    this.view.dispatch({
      effects: [
        setErrorMark.of({ from: start, to }),
        EditorView.scrollIntoView(start, { y: 'nearest' }),
      ],
    });
  }

  /** Replaces the whole document (used for generated text). */
  setText(text: string): void {
    this.view.dispatch({ changes: { from: 0, to: this.view.state.doc.length, insert: text } });
  }

  focus(): void {
    this.view.focus();
  }

  destroy(): void {
    this.view.destroy();
  }

  #reconfigure(): void {
    this.view.dispatch({
      effects: [
        this.#indent.reconfigure(this.#indentExt()),
        this.#wrap.reconfigure(this.#wrapExt()),
        this.#gutter.reconfigure(this.#gutterExt()),
        this.#brackets.reconfigure(this.#bracketsExt()),
        this.#language.reconfigure(this.#languageExt()),
        this.#readOnly.reconfigure(this.#readOnlyExt()),
      ],
    });
  }

  #indentExt(): Extension {
    const { tabSize, indentWithTabs } = this.#options;
    return [
      EditorState.tabSize.of(tabSize),
      indentUnit.of(indentWithTabs ? '\t' : ' '.repeat(tabSize)),
    ];
  }
  #wrapExt(): Extension {
    return this.#options.wordWrap ? EditorView.lineWrapping : [];
  }
  #gutterExt(): Extension {
    return this.#options.lineNumbers
      ? [lineNumbers(), highlightActiveLineGutter(), foldGutter()]
      : [];
  }
  #languageExt(): Extension {
    return sql({
      dialect: DIALECTS[this.#options.dialect ?? 'postgres'] ?? PostgreSQL,
      upperCaseKeywords: true,
    });
  }
  #readOnlyExt(): Extension {
    return EditorState.readOnly.of(this.#options.readOnly ?? false);
  }
  #bracketsExt(): Extension {
    return this.#options.autoCloseBrackets ? closeBrackets() : [];
  }

  #newState(doc: string): EditorState {
    return EditorState.create({
      doc,
      extensions: [
        this.#gutter.of(this.#gutterExt()),
        history(),
        drawSelection(),
        EditorState.allowMultipleSelections.of(true),
        rectangularSelection(),
        crosshairCursor(),
        indentOnInput(),
        bracketMatching(),
        this.#brackets.of(this.#bracketsExt()),
        highlightActiveLine(),
        highlightSelectionMatches(),
        search({ top: false }),
        this.#indent.of(this.#indentExt()),
        this.#wrap.of(this.#wrapExt()),
        this.#language.of(this.#languageExt()),
        this.#readOnly.of(this.#readOnlyExt()),
        errorField,
        syntaxHighlighting(highlight),
        theme,
        placeholder('Write SQL here…'),
        keymap.of([
          ...closeBracketsKeymap,
          // Mod-Enter belongs to "Run"; CodeMirror would insert a blank line.
          ...defaultKeymap.filter((binding) => binding.key !== 'Mod-Enter'),
          ...historyKeymap,
          ...searchKeymap,
          ...foldKeymap,
          indentWithTab,
        ]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && this.#current !== null) {
            this.#onChange(this.#current, update.state.doc.toString());
          }
        }),
      ],
    });
  }
}
