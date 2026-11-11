from pkg import shapes as s
from pkg.shapes import plain as aliased_plain

def through_module_alias():
    return s.plain()

def through_from_alias():
    return aliased_plain()
