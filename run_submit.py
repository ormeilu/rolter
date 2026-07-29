import os
import json

def submit():
    try:
        from tools import submit
        with open('pr-body.txt', 'r') as f:
            body = f.read()

        submit(
            title="🧪 [Add tests for UI API client functions]",
            body=body
        )
        print("Success")
    except Exception as e:
        print(f"Failed: {e}")

if __name__ == '__main__':
    submit()
