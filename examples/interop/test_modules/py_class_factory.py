# py_class_factory.py — 関数の中でクラスを作る Python のコード（フェーズ10 10-16）。
#
# 使う側: examples/interop/py_class_factory.ar / py_class_factory_error.ar
#
# ⚠ クラスを作って返す関数（make_counter_class・add_greeting・tagged_with）は **Python から呼ぶときだけ**
#   使える。Arrow から直接呼ぶ・デコレータに使うのは静的エラー（作られるクラスの型を Arrow は知り得ない）。


class Base:
    def describe(self):
        return "base"


def make_counter_class(step):
    # 関数の中のクラス。メソッドは関数の引数 step を閉包として捕まえ、
    # 基底には最上位のクラス Base を使う。
    class Counter(Base):
        def __init__(self, start):
            self.value = start

        def tick(self):
            self.value = self.value + step
            return self.value

    return Counter


def count_by(step, times):
    # ファクトリを Python の中で使い切る（Arrow から呼べる）。
    Counter = make_counter_class(step)
    c = Counter(0)
    i = 0
    while i < times:
        c.tick()
        i = i + 1
    return c.describe() + " " + str(c.value)


def chain_length(n):
    # メソッドがクラス自身を名前で引く（Link を返す）。
    class Link:
        def __init__(self, depth):
            self.depth = depth

        def next(self):
            return Link(self.depth + 1)

    link = Link(0)
    i = 0
    while i < n:
        link = link.next()
        i = i + 1
    return link.depth


def pick(flag):
    # if / else の中のクラス（同じ名前を 2 つの枝で定義する）。
    if flag:
        class Answer:
            def text(self):
                return "yes"
    else:
        class Answer:
            def text(self):
                return "no"
    return Answer().text()


def collect(values):
    # メソッドが関数のローカルのリストを閉包として捕まえて書き換える（同じリストを指す）。
    seen = []

    class Recorder:
        def record(self, v):
            seen.append(v)

    r = Recorder()
    for v in values:
        r.record(v)
    return len(seen)


def add_greeting(cls):
    # クラスデコレータ: 受け取ったクラスを基底にした派生クラスを返す（基底が関数の引数）。
    class WithGreeting(cls):
        def greet(self):
            return "hello from " + self.name

    return WithGreeting


@add_greeting
class Person:
    def __init__(self, name):
        self.name = name


def tagged_with(tag):
    # 引数を取るデコレータ（クラスを作る関数を返す）。
    def deco(cls):
        class Tagged(cls):
            def tag(self):
                return tag

        return Tagged

    return deco


@tagged_with("v1")
class Release:
    def __init__(self):
        self.ok = True
