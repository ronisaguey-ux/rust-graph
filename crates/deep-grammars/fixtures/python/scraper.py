"""Fetch a list of pages politely and collect their titles."""

import time
from urllib.parse import urljoin, urlparse

import requests
from bs4 import BeautifulSoup

HEADERS = {"User-Agent": "rs-systems-crawler/1.0 (+contact@example.org)"}


def fetch(session, url, timeout=10.0):
    response = session.get(url, headers=HEADERS, timeout=timeout)
    response.raise_for_status()
    return response.text


def title_of(html):
    soup = BeautifulSoup(html, "html.parser")
    node = soup.find("title")
    return node.get_text(strip=True) if node else ""


def crawl(start, limit=25, delay=0.5):
    host = urlparse(start).netloc
    seen, found, queue = set(), [], [start]
    session = requests.Session()
    while queue and len(found) < limit:
        url = queue.pop(0)
        if url in seen:
            continue
        seen.add(url)
        try:
            html = fetch(session, url)
        except requests.RequestException as error:
            print(f"skip {url}: {error}")
            continue
        found.append((url, title_of(html)))
        for anchor in BeautifulSoup(html, "html.parser").find_all("a", href=True):
            target = urljoin(url, anchor["href"])
            if urlparse(target).netloc == host and target not in seen:
                queue.append(target)
        time.sleep(delay)
    return found


if __name__ == "__main__":
    for address, title in crawl("https://example.org/"):
        print(address, "-", title)
