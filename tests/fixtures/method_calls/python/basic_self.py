class Processor:
    def transform(self, data):
        return self.validate(data)

    def validate(self, data):
        return data is not None

    def unused_helper(self):
        pass


def main():
    p = Processor()
    p.transform("test")
