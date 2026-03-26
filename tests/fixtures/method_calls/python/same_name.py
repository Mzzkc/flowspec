class Encoder:
    def process(self, data):
        return self.validate(data)

    def validate(self, data):
        return len(data) > 0


class Decoder:
    def process(self, data):
        return self.validate(data)

    def validate(self, data):
        return data != ""


def main():
    Encoder().process("test")
    Decoder().process("test")
