# rumorph-dict-ru

Словарь русского языка для морфологического анализатора [ruMorph](https://github.com/SergeyShk/ruMorph):
около 3,1 млн словоформ в формате ruMorph. Крейт подключается к `rumorph` feature `bundled`
(по умолчанию): `rumorph::Dictionary::bundled()`.

## Лицензия

Словарь собран из словаря [OpenCorpora](https://opencorpora.org), ревизия 417150, в сборке пакета
pymorphy3-dicts-ru 2.4.417150.4580142, вместе с его оценками P(тег | словоформа) по корпусу
OpenCorpora. Данные OpenCorpora распространяются по лицензии
[CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/); словарь ruMorph - переработка этих
данных и распространяется по той же лицензии.
