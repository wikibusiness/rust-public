from lzstring_optimized import lzstring_optimized

domains = ["appsealing.com","nowsecure.com","promon.co","i-sprint.com","boloro.com","guardsquare.com","certosoftware.com","cloudmask.com","beconnect.ai"];
compressed = lzstring_optimized.compress_to_base64(
    'filters={"companiesAnyOfV1":{"companies":["'
    + '","'.join(domains)
    + '"],"enabled":true}}'
)

print("http://localhost:3000/search/companies?" + compressed + "=")