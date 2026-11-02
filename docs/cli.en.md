# The rumorph command

``` bash
rumorph [options] [file ...]
```

The command analyzes text from files or the standard input and writes lemmas, parts of speech and [Universal Dependencies](https://universaldependencies.org/) features in one of five formats. It is installed with the package (`pip install rumorph`) and also runs as `python -m rumorph`; without Python, `cargo install rumorph --features cli` installs it. The Rust core does the analysis and the output in one thread: about 100 thousand tokens per second with [context](analyzer.md#analyze_sentence) and about a million without it (`--no-context`).

``` bash
echo "Мы стали друзьями в ковиде." | rumorph
```

```
мы стать друг в ковид .
```

## Options { #options }

| Option | Default | Description |
| :----- | :------ | :---------- |
| `-f`, `--format` | `lemmas` | [Output format](#formats): `lemmas`, `conllu`, `tsv`, `jsonl`, `text` |
| `-i`, `--input` | `text` | [Input](#input): `text`, `tokens`, `conllu` |
| `-d`, `--dictionary` | the package dictionary | A ruMorph dictionary file |
| `-w`, `--words` | | A file of [own words](analyzer.md#words): a word and its sample separated by a tab on each line, plus the part of speech if the sample has several; spaces around fields are ignored, empty lines and lines starting with `#` are skipped. The option can be repeated |
| `-o`, `--output` | the standard output | The result file; `-` is the standard output |
| `--no-context` | | Analyze each word without its neighbours: analyses by dictionary frequencies, as with [`analyze`](analyzer.md#analyze), several times faster |
| `-h`, `--help` | | Show the help |
| `-V`, `--version` | | Show the version |

Files are read one after another; without files and in place of `-` the standard input is read; the input is in UTF-8: a file in another encoding (cp1251, UTF-16) is not analyzed, and a single line with invalid bytes is analyzed with U+FFFD in their place. An option value is written separately or attached: `-f conllu`, `-fconllu`, `--format=conllu`. Text is analyzed by paragraphs: a paragraph is written as soon as a blank line or the end of the input closes it - on a terminal at once, and to a pipe or a file whenever the command waits for the next input line; it can be kept open and fed text paragraph by paragraph. The inputs `tokens` and `conllu` are written sentence by sentence.

In Windows PowerShell 5.1 the `>` redirection and a pipeline (`Get-Content text.txt | rumorph`) corrupt Cyrillic: pass the file as an argument and write the result with the `-o` option (in PowerShell 7 they work too). Text passed with `echo` in cmd comes in the console encoding: run `chcp 65001` first. cmd and PowerShell do not expand file patterns (`*.txt`).

## Input { #input }

- `text` - text: a blank line ends a paragraph. Tokens and sentences are found by the segmenter [anySeg](https://github.com/SergeyShk/anySeg) following the conventions of Universal Dependencies on which the context model is trained: hyphenated words by the UD guidelines («кто-то», «Ростова-на-Дону» is five tokens), numbers, dates and times («3,14», «12.10.2026», «12:30»), abbreviations and initials with their period («т. е.», «А. С.»), addresses, mentions and emoji; stress marks and soft hyphens stay in the token. A sentence may run across a line break: anySeg tells a line break inside a sentence from one of the author. A text without blank lines is analyzed in pieces of about 512 KB that end at a sentence boundary, and a line longer than a piece is split at a space: memory does not grow with the length of the input.
- `tokens` - one token per line, an empty line separates sentences; spaces around the token and everything after a tab are ignored, so a table with the token in the first column works too. Analyze a list of separate words with `--no-context`: otherwise the model takes neighbouring words for a sentence.
- `conllu` - [CoNLL-U](https://universaldependencies.org/format.html): the LEMMA, UPOS and FEATS columns are filled, while the other columns, comments, multiword tokens and empty nodes stay as they are.

``` bash
printf '1\tМы\t_\t_\t_\t_\t2\tnsubj\t_\t_\n2\tстали\t_\t_\t_\t_\t0\troot\t_\t_\n' | rumorph -i conllu -f conllu
```

```
1	Мы	мы	PRON	_	Case=Nom|Number=Plur|Person=1|PronType=Prs	2	nsubj	_	_
2	стали	стать	VERB	_	Aspect=Perf|Mood=Ind|Number=Plur|Tense=Past|VerbForm=Fin|Voice=Act	0	root	_	_
```

## Output formats { #formats }

All formats except `jsonl` take the first analysis of a token - the one chosen by the context model from the neighbouring words (a sentence longer than 256 tokens is analyzed in parts of 256, as with [`analyze_sentence`](analyzer.md#analyze_sentence)), and with `--no-context` the most probable one by the dictionary; `jsonl` writes all analyses in this order. A token without analyses gets the lowercase token as the lemma and the part of speech `X`.

### lemmas

Lemmas separated by spaces, a line per sentence; paragraphs are separated by a blank line.

### conllu

CoNLL-U as anySeg writes it: the number, token, lemma, part of speech and features, the `sent_id` and `text` comments. For the `text` input a file starts with the `newdoc` comment and a paragraph with `newpar`; MISC has `SpaceAfter=No` on a token not followed by a space in its paragraph and `SpacesAfter` when the space is not a single one (`SpacesAfter=\n` is a line break). The `text` comment is built from the tokens and the spaces between them on one line, without invisible characters; for the `tokens` input it is the tokens separated by spaces. Tokens, lemmas and the text are in Unicode NFC, as CoNLL-U requires.

``` bash
echo "Мы стали друзьями в ковиде." | rumorph -f conllu
```

```
# newdoc
# newpar
# sent_id = 1
# text = Мы стали друзьями в ковиде.
1	Мы	мы	PRON	_	Case=Nom|Number=Plur|Person=1|PronType=Prs	_	_	_	_
2	стали	стать	VERB	_	Aspect=Perf|Mood=Ind|Number=Plur|Tense=Past|VerbForm=Fin|Voice=Act	_	_	_	_
3	друзьями	друг	NOUN	_	Animacy=Anim|Case=Ins|Gender=Masc|Number=Plur	_	_	_	_
4	в	в	ADP	_	_	_	_	_	_
5	ковиде	ковид	NOUN	_	Animacy=Inan|Case=Loc|Gender=Masc|Number=Sing	_	_	_	SpaceAfter=No
6	.	.	PUNCT	_	_	_	_	_	_
```

### tsv

The token, lemma, part of speech, features and probability of the analysis separated by tabs, without a header; an empty line separates sentences.

``` bash
echo "Мы стали друзьями." | rumorph -f tsv
```

```
Мы	мы	PRON	Case=Nom|Number=Plur|Person=1|PronType=Prs	1
стали	стать	VERB	Aspect=Perf|Mood=Ind|Number=Plur|Tense=Past|VerbForm=Fin|Voice=Act	0.9997926079645355
друзьями	друг	NOUN	Animacy=Anim|Case=Ins|Gender=Masc|Number=Plur	1
.	.	PUNCT	_	1
```

### jsonl

[JSON Lines](https://jsonlines.org/): a sentence per line - the text (when the input has it) and the tokens with all their analyses: the lemma, part of speech, features, labels, probability and [source of the analysis](analysis.md). Below is one line of output formatted for reading, with two of the six analyses of «стали».

``` bash
echo "Мы стали" | rumorph -f jsonl
```

``` json
{
  "text": "Мы стали",
  "tokens": [
    {"word": "Мы", "analyses": [
      {"lemma": "мы", "pos": "PRON", "feats": {"Case": "Nom", "Number": "Plur", "Person": "1", "PronType": "Prs"},
       "labels": [], "prob": 1, "source": "dictionary"}
    ]},
    {"word": "стали", "analyses": [
      {"lemma": "стать", "pos": "VERB", "feats": {"Aspect": "Perf", "Mood": "Ind", "Number": "Plur", "Tense": "Past", "VerbForm": "Fin", "Voice": "Act"},
       "labels": ["intr"], "prob": 0.9997641726802856, "source": "dictionary"},
      {"lemma": "сталь", "pos": "NOUN", "feats": {"Animacy": "Inan", "Case": "Acc", "Gender": "Fem", "Number": "Plur"},
       "labels": [], "prob": 0.0001246478260593029, "source": "dictionary"}
    ]}
  ]
}
```

### text

The source text as is - with its line breaks, spaces and punctuation - where only «е» in a token is replaced with «ё» where the chosen analysis writes it, in the case of the token. This way a text without «ё» gets it by context: «все» becomes «всё» or stays «все», «узнаем» becomes «узнаём» or stays «узнаем». Without context (`--no-context`) «ё» is put only where all the analyses of the token write it: «все» and «звезды» stay as they are. A token with a stress mark or a soft hyphen is not changed, and a BOM at the start of a file stays. The format is for the input `text` only.

``` bash
echo "Осел стоит у елки, а еще ее ждет еж." | rumorph -f text
```

```
Осёл стоит у ёлки, а ещё её ждёт ёж.
```

## Exit codes { #exit-codes }

| Code | When |
| :--- | :--- |
| 0 | Success, including when the output is closed before the end (`rumorph text.txt \| head`) |
| 1 | An input file is missing, cannot be read or is not in UTF-8, a line is not in UTF-8 (it is analyzed with U+FFFD), an invalid CoNLL-U line; the other files are still analyzed |
| 1 | The dictionary cannot be read, the own words file has an error (it is missing or not in UTF-8, an invalid line, a word does not inflect like its sample), the output file cannot be created or is an input file, the dictionary or the own words file; nothing is analyzed |
| 1 | A write error; the analysis stops |
| 2 | Invalid options |

Error messages go to the standard error.
