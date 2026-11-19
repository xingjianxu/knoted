"""Verify persistent browser credentials and notes against the isolated NixOS guest."""

import base64
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from uuid import uuid4

from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait


def request(browser, method, path, body=None):
    return browser.execute_async_script(
        """
const [method, path, body, finish] = arguments;
const csrf = document.cookie.split('; ').find(v => v.startsWith('knoted_csrf='))?.slice(12);
const headers = csrf ? {'X-CSRF-Token': csrf} : {};
const config = {method, headers};
if (body !== null) {
  headers['Content-Type'] = 'application/json';
  config.body = JSON.stringify(body);
}
fetch('/api/v1' + path, config).then(async response => {
  if (response.headers.get('content-type')?.includes('application/json')) {
    finish({status: response.status, data: await response.json()});
  } else {
    const bytes = new Uint8Array(await response.arrayBuffer());
    const digest = await crypto.subtle.digest('SHA-256', bytes);
    finish({status: response.status,
            digest: Array.from(new Uint8Array(digest), b => b.toString(16).padStart(2, '0')).join('')});
  }
}).catch(() => finish({status: 0}));
""",
        method,
        path,
        body,
    )


def upload(browser, image):
    return browser.execute_async_script(
        """
const [image, key, finish] = arguments;
const csrf = document.cookie.split('; ').find(v => v.startsWith('knoted_csrf='))?.slice(12);
const file = new Blob([Uint8Array.from(atob(image), c => c.charCodeAt(0))], {type: 'image/png'});
const body = new FormData();
body.set('file', file, 't70-fixture.png');
fetch('/api/v1/assets', {method: 'POST',
  headers: {'X-CSRF-Token': csrf, 'Idempotency-Key': key}, body}).then(async response =>
  finish({status: response.status, id: (await response.json()).id})
).catch(() => finish({status: 0}));
""",
        base64.b64encode(image).decode(),
        str(uuid4()),
    )


def click(browser, name):
    selector = (By.XPATH, f'//button[normalize-space(.) = "{name}"]')
    WebDriverWait(browser, 25).until(
        lambda driver: driver.find_element(*selector)
    ).click()


def logged_in(browser):
    WebDriverWait(browser, 25).until(
        lambda driver: request(driver, "GET", "/auth/session")["status"] == 200
    )


def login(browser, origin):
    browser.get(origin + "/login")
    assert browser.execute_script("return window.isSecureContext")
    assert request(browser, "GET", "/auth/session")["status"] == 401
    click(browser, "使用 Passkey 登录")
    logged_in(browser)


def verify_note(browser, record):
    result = request(browser, "GET", "/notes/" + record["note"])
    assert result["status"] == 200
    assert result["data"]["title"] == "T70 虚拟机依赖降级笔记"
    assert "T70 业务数据在依赖停止期间可写" in json.dumps(
        result["data"], ensure_ascii=False
    )
    asset = request(browser, "GET", "/assets/" + record["image"] + "/content")
    assert asset["status"] == 200
    assert asset["digest"] == record["digest"]
    keys = request(browser, "GET", "/auth/passkeys")
    assert keys["status"] == 200
    assert [item["id"] for item in keys["data"]["items"]] == [record["passkey"]]


def seed(browser, authenticator, origin, package, image_file):
    browser.get(origin + "/bootstrap")
    assert browser.execute_script("return window.isSecureContext")
    token_output = subprocess.check_output(
        [
            "runuser",
            "-u",
            "knoted",
            "--",
            package + "/bin/knoted-api",
            "admin",
            "token",
            "--purpose",
            "bootstrap",
            "--config",
            "/run/t70-private/app.toml",
        ],
        text=True,
    )
    match = re.search(r"^token: (.+)$", token_output, re.MULTILINE)
    if not match:
        raise RuntimeError("fixture bootstrap token was not issued")
    browser.find_element(
        By.XPATH, '//label[contains(., "一次性令牌")]/following::input[1]'
    ).send_keys(match.group(1))
    browser.find_element(
        By.XPATH, '//label[contains(., "凭据名称")]/following::input[1]'
    ).send_keys("T70 合成 Passkey")
    click(browser, "注册 Passkey")
    logged_in(browser)
    keys = request(browser, "GET", "/auth/passkeys")
    assert keys["status"] == 200
    assert len(keys["data"]["items"]) == 1
    image_bytes = Path(image_file).read_bytes()
    image = upload(browser, image_bytes)
    assert image["status"] == 201
    note = request(
        browser,
        "POST",
        "/notes",
        {
            "title": "T70 虚拟机依赖降级笔记",
            "parent_id": None,
            "document": {
                "blocks": [
                    {
                        "id": "text",
                        "type": "paragraph",
                        "content": [
                            {"type": "text", "text": "T70 业务数据在依赖停止期间可写"},
                        ],
                    },
                    {"id": "image", "type": "image", "asset_id": image["id"]},
                ]
            },
        },
    )
    assert note["status"] == 201
    assert (
        request(browser, "POST", "/notes/" + note["data"]["id"] + "/snapshots")[
            "status"
        ]
        == 201
    )
    credentials = browser.execute_cdp_cmd(
        "WebAuthn.getCredentials",
        {
            "authenticatorId": authenticator,
        },
    )["credentials"]
    assert len(credentials) == 1
    return {
        "note": note["data"]["id"],
        "image": image["id"],
        "digest": hashlib.sha256(image_bytes).hexdigest(),
        "passkey": keys["data"]["items"][0]["id"],
        "credentials": credentials,
    }


def main():
    phase, package, image_file, pin = sys.argv[1:]
    os.umask(0o077)
    if not Path("/run/t70-browser-vm").is_file():
        raise RuntimeError("requires the isolated T70 browser VM")
    record_file = Path("/root/t70-browser-fixture.json")
    origin = "https://t70.knoted.test:16601"
    options = webdriver.ChromeOptions()
    options.binary_location = "/run/current-system/sw/bin/chromium"
    for option in (
        "--headless",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--no-proxy-server",
        "--ignore-certificate-errors-spki-list=" + pin,
        "--host-resolver-rules=MAP t70.knoted.test 127.0.0.1",
    ):
        options.add_argument(option)
    driver = webdriver.Chrome(
        service=webdriver.ChromeService("/run/current-system/sw/bin/chromedriver"),
        options=options,
    )
    try:
        driver.execute_cdp_cmd("WebAuthn.enable", {})
        authenticator = driver.execute_cdp_cmd(
            "WebAuthn.addVirtualAuthenticator",
            {
                "options": {
                    "protocol": "ctap2",
                    "transport": "internal",
                    "hasResidentKey": True,
                    "hasUserVerification": True,
                    "isUserVerified": True,
                    "automaticPresenceSimulation": True,
                },
            },
        )["authenticatorId"]
        if phase == "before-reboot":
            record = seed(driver, authenticator, origin, package, image_file)
            verify_note(driver, record)
            record_file.write_text(json.dumps(record))
        else:
            assert phase == "after-reboot"
            record = json.loads(record_file.read_text())
            for credential in record["credentials"]:
                driver.execute_cdp_cmd(
                    "WebAuthn.addCredential",
                    {
                        "authenticatorId": authenticator,
                        "credential": credential,
                    },
                )
            login(driver, origin)
            verify_note(driver, record)
            record_file.unlink()
        print(
            f"T70 Chromium {phase}: HTTPS, Passkey, authenticated note and image verified"
        )
    finally:
        driver.quit()


if __name__ == "__main__":
    main()
