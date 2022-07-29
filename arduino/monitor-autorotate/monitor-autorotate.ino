#include <Arduino_LSM9DS1.h>
#define PIN_LED   13
#define LED_RED   22     
#define LED_BLUE  24     
#define LED_GREEN 23
#define LED_PWR   25

/*#define SYN 'a'
#define ACK 'b'
#define NAK 'c'
#define ENQ 'd'
#define DC1 'e'
#define DC2 'f'
#define DC3 'g'
#define DC4 'h'//*/

#define SYN 0x16
#define ACK 0x06
#define NAK 0x15
#define ENQ 0x05
#define DC1 0x11
#define DC2 0x12
#define DC3 0x13
#define DC4 0x14//*/

#define LANDSCAPE 0
#define PORTRAIT 1,
#define LANDSCAPE_FLIPPED 2,
#define PORTRAIT_FLIPPED 3

bool  running = false;
float offset          =  0;
int   current_ori     =  0;
int   angle_threshold = 65;
int   angle_limit     = 65+90;

void setup() {
  Serial.begin(9600);
  pinMode(LED_RED,   OUTPUT);
  pinMode(LED_BLUE,  OUTPUT);
  pinMode(LED_GREEN, OUTPUT);
  //pinMode(LED_PWR, OUTPUT);
  pinMode(PIN_LED,   OUTPUT);

  IMU.begin();
  calibrate_IMU();
  IMU.end();
}

float x, y, z, angle;
void loop() {
  if (running && IMU.accelerationAvailable()) {
    IMU.readAcceleration(x, y, z);
    angle = deviation(x,y)-offset;

    //Serial.print("\n");
    //Serial.println(angle);
    if (angle > angle_threshold) {
      if (angle >= angle_limit) {
        current_ori = (current_ori+2)%4;
        offset += 180;
      } else {
        if (current_ori == LANDSCAPE) {current_ori = PORTRAIT_FLIPPED;}
        else {current_ori -= 1;}
        offset += 90;
      }
      serial_changeori(DC1+current_ori);
    } else if (-angle > angle_threshold) {
      if (angle >= angle_limit) {
        current_ori = (current_ori+2)%4;
        offset -= 180;
      } else {
        current_ori = (current_ori+1)%4;
        serial_changeori(DC1+current_ori);
        offset -= 90;
      }
    } //else {Serial.write(NAK);}
  }

  if (Serial.available() > 0) {
    //Serial.write(ACK);
    //int r = Serial.read();
    //Serial.write(r);
    switch (Serial.read()) {
      case DC1:
        digitalWrite(PIN_LED, HIGH);
        Serial.write(ENQ);
        
        digitalWrite(LED_RED,   HIGH);
        digitalWrite(LED_BLUE,   LOW);
        digitalWrite(LED_GREEN, HIGH);
        
        while (Serial.available() < 2) {delay(1);}
        current_ori = Serial.read();
        angle_threshold = Serial.read();
        angle_limit = angle_threshold+90;
        //Serial.read(); Serial.read();

        running = true;
        IMU.begin();
        
        digitalWrite(LED_BLUE,  HIGH);
        Serial.write(ACK);
        
        break;
      case DC2:
        digitalWrite(PIN_LED, LOW);
        running = false;
        IMU.end();
        Serial.write(ACK);
        break;
      case ENQ:
        if (running) {Serial.write(ACK);}
        else         {Serial.write(NAK);}
        break;
      case SYN:
        Serial.write(ACK);
        break;
    }
    //receivedChar = Serial.read();    
  }
  //delay(10);
  /*if (IMU.accelerationAvailable()) {
    float x, y, z,  angle;
    IMU.readAcceleration(x, y, z);

    angle = deviation(x,y);
    Serial.print(angle);
    Serial.print('\t');
    Serial.print(offset);
    Serial.print('\t');
    Serial.println(angle-offset);
  }*/
}

bool serial_changeori(int ori) {
  Serial.write(ori);
  delay(100);
  
  while (Serial.available() == 0) {
    delay(100);
    Serial.write(ori);
  }

  return Serial.read() == ACK;
}

void calibrate_IMU() {
  digitalWrite(LED_RED,    LOW);
  digitalWrite(LED_BLUE,  HIGH);
  digitalWrite(LED_GREEN, HIGH);
  
  float x, y, z;
  int actual_i = 0;
  for (int i = 0; i < 10000; ++i) {
    if (i%39 == 0) {analogWrite(LED_RED/39-1, i);}
    if (IMU.accelerationAvailable()) {
      actual_i++;
      IMU.readAcceleration(x, y, z);
      offset += deviation(x,y);
    }
  }
  offset /= actual_i;
  
  digitalWrite(LED_RED, HIGH);
  //Serial.println("\ncalib done");
}

float deviation(float a, float b) {
  float deg = atan(a/b)*RAD_TO_DEG;
  if (b <= 0) {
    if (a >= 0) {deg += 180;}
    else        {deg -= 180;}
  }
  return deg; 
}