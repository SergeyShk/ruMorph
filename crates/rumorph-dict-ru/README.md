# rumorph-dict-ru

Словарь русского языка для морфологического анализатора [ruMorph](https://github.com/SergeyShk/ruMorph):
около 3,8 млн словоформ в формате ruMorph, модель класса парадигмы для незнакомых слов и контекстная модель для разбора предложения. Крейт
подключается к `rumorph` feature `bundled` (по умолчанию): `rumorph::Dictionary::bundled()`.

## Лицензия

Словарь собран из словаря [OpenCorpora](https://opencorpora.org), ревизия 417258, с оценками
P(тег | словоформа) по корпусу OpenCorpora, ревизия 4580223 (лицензия
[CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/)), из существительных, имен собственных
и прилагательных [Викисловаря](https://ru.wiktionary.org), которых нет в OpenCorpora (выгрузка
01.10.2026, лицензия [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)), и из счетчиков
разметки словоформ в обучающих частях трибанков Universal Dependencies
[UD_Russian-Taiga](https://github.com/UniversalDependencies/UD_Russian-Taiga) и
[UD_Russian-GSD](https://github.com/UniversalDependencies/UD_Russian-GSD), релиз 2.19 (лицензия
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)); на обучающих и проверочных частях
этих трибанков, обучающей части [UD_Russian-Poetry](https://github.com/UniversalDependencies/UD_Russian-Poetry)
(лицензия CC BY-SA 4.0) и на корпусе OpenCorpora, ревизия 4580223 (лицензия CC BY-SA 3.0), обучена
контекстная модель; модель класса парадигмы обучена на словах самого словаря. Словарь ruMorph - переработка этих данных
и распространяется по лицензии CC BY-SA 4.0.
