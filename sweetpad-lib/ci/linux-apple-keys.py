"""Apple FCS key envelopes: RFC 9180 base HPKE, P-256/SHA256/AES256GCM.

No account cookies or credentials are used. Online key retrieval is restricted
to Apple's HTTPS distribution hosts and a small response size.
"""
import base64
import hashlib
import hmac
import json
import urllib.parse
import urllib.request

HOSTS = {"swdist.apple.com", "wkms-public.apple.com", "appldnld.apple.com", "updates.cdn-apple.com"}
_cache = {}


def validate_url(url):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "https" or parsed.hostname not in HOSTS or parsed.port not in (None,443) or parsed.username or parsed.password or parsed.fragment:
        raise ValueError("AEA key URL must use an approved Apple HTTPS distribution host; supply --aea-key-file for other sources")
    return url


class AppleRedirects(urllib.request.HTTPRedirectHandler):
    max_redirections = 3

    def redirect_request(self, request, fp, code, message, headers, url):
        validate_url(url)
        return super().redirect_request(request,fp,code,message,headers,url)


def fetch_key(url):
    validate_url(url)
    if url in _cache:
        return _cache[url]
    try:
        opener = urllib.request.build_opener(AppleRedirects())
        request = urllib.request.Request(url,headers={"User-Agent":"Sweetpad SDK import", "Accept":"application/x-pem-file, application/octet-stream"})
        with opener.open(request,timeout=20) as response:
            value = response.read(128*1024+1)
        if len(value)>128*1024:
            raise ValueError("Apple archive key response exceeds the size limit")
    except OSError:
        raise ValueError("Apple archive key download failed; supply --aea-key-file or use a Mac SDK export") from None
    if len(_cache)>=16:
        _cache.clear()
    _cache[url]=value
    return value


def metadata(data):
    result={};pos=0
    while pos<len(data):
        if pos+4>len(data):
            return {}
        length=int.from_bytes(data[pos:pos+4],"little")
        if length<5 or length>len(data)-pos:
            return {}
        entry=data[pos+4:pos+length]
        if b"\0" not in entry:
            return {}
        key,value=entry.split(b"\0",1)
        try: key=key.decode("utf-8")
        except UnicodeError: return {}
        if key in result and key.startswith("com.apple.wkms."):
            raise ValueError("Duplicate Apple key metadata")
        result[key]=value;pos+=length
        if len(result)>128:
            raise ValueError("Too many Apple key metadata entries")
    return result


def expand(key,info,size):
    value=b"";previous=b""
    for index in range(1,(size+31)//32+1):
        previous=hmac.digest(key,previous+info+bytes([index]),"sha256");value+=previous
    return value[:size]


def hpke_open(private_key,encapsulated,ciphertext,info=b"",associated=b""):
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric import ec
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    if len(encapsulated)!=65 or len(ciphertext)>4096:
        raise ValueError("Invalid Apple HPKE envelope length")
    if isinstance(private_key,bytes):
        loader=serialization.load_pem_private_key if private_key.startswith(b"-----") else serialization.load_der_private_key
        private_key=loader(private_key,password=None)
    if not isinstance(private_key,ec.EllipticCurvePrivateKey) or private_key.curve.name!="secp256r1":
        raise ValueError("Apple FCS key must use P-256")
    public=private_key.public_key().public_bytes(serialization.Encoding.X962,serialization.PublicFormat.UncompressedPoint)
    sender=ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(),encapsulated)
    dh=private_key.exchange(ec.ECDH(),sender)

    def labeled_extract(suite,salt,label,input_key):
        return hmac.digest(salt or bytes(32),b"HPKE-v1"+suite+label+input_key,"sha256")

    def labeled_expand(suite,key,label,context,size):
        return expand(key,size.to_bytes(2,"big")+b"HPKE-v1"+suite+label+context,size)

    kem=b"KEM\x00\x10"
    ephemeral=labeled_extract(kem,b"",b"eae_prk",dh)
    shared=labeled_expand(kem,ephemeral,b"shared_secret",encapsulated+public,32)
    suite=b"HPKE\x00\x10\x00\x01\x00\x02"
    psk_hash=labeled_extract(suite,b"",b"psk_id_hash",b"")
    info_hash=labeled_extract(suite,b"",b"info_hash",info)
    context=b"\x00"+psk_hash+info_hash
    secret=labeled_extract(suite,shared,b"secret",b"")
    key=labeled_expand(suite,secret,b"key",context,32)
    nonce=labeled_expand(suite,secret,b"base_nonce",context,12)
    return AESGCM(key).decrypt(nonce,ciphertext,associated)


def unwrap(auth,private_key=None):
    fields=metadata(auth)
    if "com.apple.wkms.fcs-response" not in fields:
        return None
    from cryptography.exceptions import InvalidTag
    try:
        if len(fields["com.apple.wkms.fcs-response"])>16384:
            raise ValueError("Apple FCS response exceeds the size limit")
        response=json.loads(fields["com.apple.wkms.fcs-response"])
        encapsulated=base64.b64decode(response["enc-request"],validate=True)
        ciphertext=base64.b64decode(response["wrapped-key"],validate=True)
        if private_key is None:
            private_key=fetch_key(fields["com.apple.wkms.fcs-key-url"].decode("utf-8"))
        key=hpke_open(private_key,encapsulated,ciphertext)
        if len(key)!=32:
            raise ValueError("Apple FCS envelope did not contain a 32-byte archive key")
        return key
    except (KeyError,TypeError,UnicodeError,InvalidTag):
        raise ValueError("Invalid Apple FCS key envelope") from None
