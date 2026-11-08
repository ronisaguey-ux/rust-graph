import asyncio
from contextlib import asynccontextmanager


@asynccontextmanager
async def connection(url):
    conn = await asyncio.open_connection(url, 6379)
    try:
        yield conn
    finally:
        conn[1].close()
        await conn[1].wait_closed()


async def consume(queue, workers=4):
    async def one(worker_id):
        while True:
            item = await queue.get()
            if item is None:
                queue.task_done()
                return
            try:
                await handle(worker_id, item)
            except Exception as exc:
                print(f"worker {worker_id} failed on {item!r}: {exc!r}")
            finally:
                queue.task_done()

    tasks = [asyncio.create_task(one(index)) for index in range(workers)]
    await queue.join()
    for _ in tasks:
        await queue.put(None)
    await asyncio.gather(*tasks)


async def handle(worker_id, item):
    await asyncio.sleep(item.get("delay", 0))
    if item.get("kind") == "boom":
        raise RuntimeError("requested failure")


async def main(urls):
    queue = asyncio.Queue()
    async with connection(urls[0]):
        for index, item in enumerate({"delay": index % 3} for index in range(10)):
            await queue.put(item)
        await consume(queue)


if __name__ == "__main__":
    asyncio.run(main(["localhost"]))
