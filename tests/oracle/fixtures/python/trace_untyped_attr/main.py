from worker import Worker

class Runner:
    def __init__(self):
        self.x = Worker()

    def run(self):
        return self.x.do_job()
