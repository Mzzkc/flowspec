"""Bounded AST calibration for dispatch shapes in the commission."""

def plain():
    return 1

def decorate(fn):
    return fn

@decorate
def decorated():
    return plain()

class Base:
    def ping(self):
        return 1

class Child(Base):
    def ping(self):
        return super().ping()

class Inherited(Base):
    pass

class One:
    def same(self):
        return 1

class Two:
    def same(self):
        return 2

class Modes:
    @classmethod
    def class_run(cls):
        return Child().ping()

    @staticmethod
    def static_run():
        return One().same()

    @property
    def value(self):
        return 3

def use(one: One, unknown):
    one.same()
    unknown.same()
    Two().same()
    Modes.class_run()
    Modes.static_run()
    Inherited().ping()
    _ = Modes().value
    callback = plain
    table = {'run': plain}
    return callback, table
