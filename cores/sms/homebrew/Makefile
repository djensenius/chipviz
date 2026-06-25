ROM = chipviz-sms.sms

all: $(ROM)
.PHONY: all clean

$(ROM): ../../../shared/tools/build_homebrew.py
	python3 ../../../shared/tools/build_homebrew.py --target sms --output .
	rm -f homebrew-status.json

clean:
	rm -f $(ROM) homebrew-status.json
