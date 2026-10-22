# Text in data packs

A data pack adds or replaces the messages the game shows: the dialogue boxes with a portrait and the narration
boxes. It does this with a messages file, `text/messages.toml`, that the pack's manifest names. The game does not
change: each message is compiled into the strings and the lists the game's own message engine already reads, so
the text keeps the game's timing, its windows and its pages. The engine's data is described in
`docs/re/data/text-messages.md`.

## The manifest

```toml
[[messages]]
file = "text/messages.toml"
```

A pack may name several messages files; each is read in the order the manifest gives them.

## The messages file

A file holds `[[message]]` tables. Each table is one message:

```toml
[[message]]
id = 400
window = "dialogue"
portrait = 5
pages = [
  ["\"You came far.", "Dangerous ahead.\""],
  ["\"Take care.\""],
]
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | The message number, 0 to 16383. Numbers from 368 on are new messages; a lower number replaces the field message with that number (see "Battles" below). |
| `window` | yes | `"dialogue"` for the box with a portrait, `"narration"` for the narration box. |
| `portrait` | no | A portrait number, 0 to 255. The game writes it at the start of each page of the message. |
| `pages` | yes | The pages, in order. Each page is a list of lines, shown together. |

A message has at least one page, and a page has at least one line. Any other key in a message table is an error.

## Characters

The English font has tiles for these characters, and only these:

- the space;
- the letters `a` to `z` and `A` to `Z`;
- the digits `0` to `9`;
- `! ? . , ' -` and the ellipsis `…`;
- the quotation mark `"`.

A quotation mark opens a speech when it is the first character of a line or follows a space; otherwise it closes
one. Any other character, such as `;`, `:`, `(`, `)`, `/`, `&` or `%`, is an error.

Three kinds of token stand for things the game fills in while the message is shown:

| Token | Shows |
|---|---|
| `{name:1}`, `{name:2}`, `{name:3}` | a name from name slot 1, 2 or 3 (a unit, an object or an attack) |
| `{number:1}`, `{number:2}`, `{number:3}` | a number from number slot 1, 2 or 3 |
| `{portrait N}` | a portrait change in the middle of a page (N from 0 to 255) |

A pack chooses which slot to show, not the name or the number itself.

## Budgets

The windows hold a fixed number of cells. A cell is one character, a space included.

| Window | Cells per line | Lines per page |
|---|---|---|
| `dialogue` | 20 | 5 |
| `narration` | 25 | 4 |

- A name token counts 15 cells, a number token 8 cells (the most each can take), and a portrait token none.
- On a dialogue page, every line after the first starts with a space, which is a cell: it counts toward the 20.
  The first line is not moved by the compiler; the game writes it one cell to the left of the others when a portrait
  is shown.

A line or a page over its budget is an error. The compiler breaks no line by itself: the author writes the lines.

## What the compiler writes

For each message the compiler writes one string and one list:

- The string is the pages. The first line of a page is written first, the lines after it follow with `FF` between
  them. A page ends with `FE FF`, and the last page of a message ends with `FF FF`. A page starts with the portrait
  token when `portrait` is set, and every page repeats it.
- The list is three words: the table (`D002` for a dialogue, `D003` for a narration), the string's index in that
  table, and `FFFF`, the end of the list.

The index is not written by the author. The game's tables of strings keep their original entries, and the new
strings follow them: the dialogue strings start at index 366, the narration strings at index 167, up to index 4095.
The packs of a set are numbered in the order of their ids; within a pack, the messages take the indices in the order
their messages files and the messages of each file are given. So the same set of packs always gets the same indices,
whatever the order it is loaded in.

## Errors

Each error names the message, the page and the line where it is, in plain words. The ones a messages file can give:

- the file is not valid TOML, or a key or a window name is not one of the ones above;
- the message id is above 16383;
- the message has no page, or a page has no line;
- a page has more lines than its window holds;
- a line has a character the font has no tile for, and the error gives the character and its column;
- a token is not one of the tokens above, or a `{` has no `}`;
- a line is wider than its window.

A pack with an error in its messages file is not loaded, and the checks say why. Two packs may not give the same
message number, and a pack may not give the same message number twice.

## Battles

In a battle the game reads its messages through a second list that uses the same message numbers. A pack message
replaces only the field's message of its number: the battle's messages are never changed.

## Not covered

This format covers the messages of the two windows. It does not change the names of units and attacks, the captions
of the story's transitions, the ending's texts, or the text of the menus.
