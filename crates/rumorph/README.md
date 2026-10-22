# rumorph

Быстрый морфологический анализатор русского языка на Rust - ядро библиотеки
[ruMorph](https://github.com/SergeyShk/ruMorph) для Python.

- Разборы словоформы по словарю OpenCorpora: лемма, часть речи и признаки
  [Universal Dependencies](https://universaldependencies.org/) в соглашениях трибанка SynTagRus,
  вероятность разбора.
- Незнакомые слова: числа, знаки, латиница, сокращения, слова через дефис и с приставками,
  угадывание по окончанию.
- Формы лексемы, форма с заданными признаками и согласование с количеством.
- Словарь отображается в память или читается из данных программы без копий: загрузка около 3 мс,
  разбор словоформы - около 0,6 мкс.

## Установка

```toml
[dependencies]
rumorph = "0.3"
```

Словарь OpenCorpora входит в крейт [`rumorph-dict-ru`](https://crates.io/crates/rumorph-dict-ru)
и подключается feature `bundled` (по умолчанию). Без нее словарь читается из файла:
`Dictionary::open`.

## Пример

```rust
use rumorph::Dictionary;

fn main() -> Result<(), rumorph::Error> {
    let dictionary = Dictionary::bundled()?;
    let analysis = &dictionary.analyze("стали")[0];
    let tag = dictionary.ud_tag(analysis.ud).unwrap();
    assert_eq!((analysis.lemma.as_str(), tag.pos()), ("стать", "VERB"));
    assert_eq!(tag.feat("Tense"), Some("Past"));

    let cat = &dictionary.analyze("кошка")[0];
    assert_eq!(dictionary.inflect(cat, &[("Case", "Gen")]).unwrap().word, "кошки");
    let ruble = &dictionary.analyze("рубль")[0];
    assert_eq!(dictionary.agree(ruble, 5).unwrap().word, "рублей");

    // Свое слово изменяется по образцу словарного; копия словаря делит отображенный файл
    let zoomers = dictionary.clone().with_words([("зумер", "геймер")])?;
    assert_eq!(zoomers.analyze("зумерами")[0].lemma, "зумер");
    Ok(())
}
```

Словарь загружается и проверяется при каждом вызове `Dictionary::bundled` и `Dictionary::open`:
создайте его один раз и разбирайте им весь текст, в том числе из нескольких потоков.

## Команда rumorph

```bash
cargo install rumorph --features cli
echo "Мы стали друзьями в ковиде." | rumorph -f conllu
```

Команда пишет леммы, части речи и признаки UD в форматах `lemmas`, `conllu`, `tsv` и `jsonl`;
параметры - в [документации](https://sergeyshk.github.io/ruMorph/cli/).

## Features

- `bundled` (по умолчанию) - словарь из крейта `rumorph-dict-ru`: `Dictionary::bundled`.
- `build` - сборка своих словарей: `Builder`.
- `cli` - команда `rumorph`.

## Лицензия

Код распространяется под лицензией MIT. Словарь в крейте `rumorph-dict-ru` собран из словаря
[OpenCorpora](https://opencorpora.org) с частотами по трибанкам UD Taiga и GSD и распространяется по
лицензии CC BY-SA 4.0.
