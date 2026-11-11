from pkg.sheet import Sheet, make

LIMIT = 3

def direct(sheet: Sheet):
    return sheet.template_variables(LIMIT)

def constructed():
    s = Sheet()
    return s.template_variables(1)

def untyped(sheet):
    return sheet.template_variables(2)

class Holder:
    def __init__(self):
        self.sheet = make()
    def go(self):
        return self.sheet.template_variables(4)
