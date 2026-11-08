import json
import logging

from flask import Blueprint, abort, jsonify, request

log = logging.getLogger(__name__)
bp = Blueprint("items", __name__, url_prefix="/api/items")

STORE = {}
NEXT_ID = {"value": 0}


def _payload():
    body = request.get_json(silent=True)
    if not isinstance(body, dict):
        abort(400, description="body must be a JSON object")
    return body


@bp.get("/")
def list_items():
    return jsonify(sorted(STORE.values(), key=lambda item: item["id"]))


@bp.post("/")
def create_item():
    body = _payload()
    name = body.get("name", "").strip()
    if not name:
        abort(422, description="name is required")
    NEXT_ID["value"] += 1
    item = {"id": NEXT_ID["value"], "name": name, "done": bool(body.get("done"))}
    STORE[item["id"]] = item
    return jsonify(item), 201


@bp.patch("/<int:item_id>")
def update_item(item_id):
    item = STORE.get(item_id)
    if item is None:
        abort(404)
    try:
        item.update(_payload())
    except (TypeError, ValueError) as error:
        log.warning("bad update for %s: %s", item_id, error)
        abort(422, description=str(error))
    return jsonify(item)
