from random import randint

def passcode2internal(passcode: str):
    notation_map = dict(zip('PNBRQKpnbrqk', range(12)))  # capital for white
    result = 0
    for i in reversed(range(6)):
        result *= 12
        result += notation_map[passcode[i]]
    return result

def internal2passcode(internal: int):
    notation_map = dict(zip(range(12), 'PNBRQKpnbrqk'))
    result = ''
    for i in range(6):
        result += notation_map[internal % 12]
        internal //= 12
    return result

def generate_random_passcode():
    return internal2passcode(randint(0, 2985983))  # kkkkkk = 2985983

if __name__ ==  '__main__':
    assert passcode2internal('PPPPPP') == 0x00000000
    assert passcode2internal('NNNNNN') == 0x0004245d
    assert passcode2internal('PnBrQk') == 0x002b4634

    assert internal2passcode(0x00000000) == 'PPPPPP'
    assert internal2passcode(0x0004245d) == 'NNNNNN'
    assert internal2passcode(0x002b4634) == 'PnBrQk'

    print(generate_random_passcode())

    s = input()
    if s.startswith('0x'):
        v = int(s[2:], base=16)
        print(internal2passcode(v))
    else:
        try:
            v = int(s)
        except ValueError:
            print(passcode2internal(s))
        else:
            print(internal2passcode(v))
